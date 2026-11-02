//! Compose deferred visualizer choices. Environments keep owning workload execution.
use super::{
    catalog::{Catalog, Launch, Visualization},
    graphics, launch,
    selection::{BrowserSettings, Domain, Environment, Options, Selection},
};
use crate::support::{movement_config, schema, uptime_config};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};
type Failure = Box<dyn std::error::Error>;
#[path = "../accounts/config.rs"]
mod accounts_config;
#[path = "../robotics/config.rs"]
mod robotics_config;

pub(super) fn configs(selection: Selection, arguments: &[String]) -> Result<(movement_config::Config, uptime_config::Config), String> {
    // Only shared settings belong before the graphical case choice. Direct-case
    // commands still expose each case's complete configuration form.
    let mut input = arguments.iter();
    while let Some(flag) = input.next() {
        match flag.as_str() {
            "--no-setup" => {}
            "--workers" | "--retention-ms" | "--checkpoint-interval" | "--query-hz" | "--duration" => {
                input.next().ok_or_else(|| format!("{flag} requires a value"))?;
            }
            _ => return Err(format!("{flag} is case-specific; select client-movement or service-uptime directly to configure it")),
        }
    }
    let arguments: Vec<_> = ["--workers".into(), "1".into()]
        .into_iter()
        .chain(arguments.iter().cloned())
        .chain(["--execution".into(), selection.execution.slug().into(), "--no-setup".into()])
        .collect();
    let movement = movement_config::Config::parse(arguments.clone())?.ok_or("Missing movement configuration")?;
    let uptime = uptime_config::Config::parse(arguments)?.ok_or("Missing uptime configuration")?;
    Ok((movement, uptime))
}
pub(super) fn definitions(selection: Selection, args: &[String]) -> Result<Vec<schema::Definition>, String> {
    let (_, config) = configs(selection, args)?;
    let mut definitions = schema::contime(config.workers, config.manual, config.retention_ms, config.checkpoint_interval);
    definitions.extend(schema::uptime(&config).into_iter().filter(|d| d.id == "--query-hz").map(|mut d| {
        d.group = "Visualizer";
        d.help = "Maximum completed snapshot query updates per second for the selected visualization.";
        d
    }));
    Ok(definitions)
}
fn catalog(root: &Path, selection: Selection, arguments: &[String]) -> Result<Catalog, String> {
    let (movement, uptime) = configs(selection, arguments)?;
    let accounts = accounts_config::Config {
        manual: uptime.manual,
        workers: uptime.workers,
        retention_ms: uptime.retention_ms,
        checkpoint_interval: uptime.checkpoint_interval,
        query_hz: uptime.query_hz,
        duration: uptime.duration,
    };
    let robotics = robotics_config::Config {
        manual: uptime.manual,
        workers: uptime.workers,
        retention_ms: uptime.retention_ms,
        checkpoint_interval: uptime.checkpoint_interval,
        query_hz: uptime.query_hz,
        duration: uptime.duration,
    };
    let settings = movement.settings()?;
    let mut launches = Vec::new();
    for visualization in Visualization::ALL {
        let arguments = match visualization {
            Visualization::Movement => movement.arguments(),
            Visualization::Uptime => uptime.arguments(),
            Visualization::Accounts => accounts.arguments(),
            Visualization::Robotics => robotics.arguments(),
        };
        let mut guest = if selection.environment == Environment::Terminal {
            graphics::native_guest(root, selection, &arguments)
        } else {
            let mut guest = launch::wasi_case_command(root, selection, &settings, Some(&arguments));
            guest.args.extend([
                "--metrics-stdio".into(),
                "--control-input".into(),
                if selection.environment == Environment::WasmThreads { "poll".into() } else { "nonblocking".into() },
            ]);
            guest
        };
        // The graphical case identity is chosen after the environment. Recipes
        // carry its slug directly instead of adding terminal menu entries.
        let domain = guest.args.iter().position(|arg| arg == "--domain").ok_or("Missing workload domain")? + 1;
        guest.args[domain] = visualization.slug().into();
        // The recipe is data only. The Bevy chooser is responsible for spawning it.
        launches.push(Launch { visualization, program: std::mem::take(&mut guest.program), args: guest.args, cwd: guest.cwd });
    }
    let catalog = Catalog { launches };
    catalog.validate()?;
    Ok(catalog)
}
fn browser_settings(config: &movement_config::Config) -> Result<crate::support::Settings, String> {
    let mut settings = config.settings()?;
    settings.query_hz = usize::try_from(config.query_hz).map_err(|_| "Query frequency exceeds host range")?;
    Ok(settings)
}
pub(super) fn run(
    root: &Path,
    selection: Selection,
    arguments: &[String],
    browser: BrowserSettings,
    options: &Options,
    cancelled: Arc<AtomicBool>,
) -> Result<(), Failure> {
    let (movement, _) = configs(selection, arguments)?;
    if selection.environment == Environment::Browser {
        return launch::browser(
            root,
            Domain::Visualizer,
            &browser_settings(&movement)?,
            None,
            None,
            browser,
            options.dry_run,
            options.no_open,
            cancelled,
        );
    }
    let mut catalog = catalog(root, selection, arguments)?;
    match selection.environment {
        Environment::Terminal => {
            let build = graphics::native_build(root);
            if options.dry_run {
                build.show();
            } else {
                build.run()?;
            }
        }
        Environment::Wasm | Environment::WasmThreads => {
            let program = launch::prepare_wasi(root, selection, options.dry_run)?;
            for launch in &mut catalog.launches {
                launch.program.clone_from(&program);
            }
        }
        Environment::Browser => unreachable!(),
    }
    graphics::present_catalog(root, &catalog, options.dry_run, &cancelled)
}
#[cfg(test)]
mod tests {
    use super::super::selection::Execution;
    use super::*;
    fn selection(environment: Environment, execution: Execution) -> Selection {
        Selection::new(environment, execution, Domain::Visualizer).unwrap()
    }
    #[test]
    fn every_host_has_separate_validated_recipes_with_case_defaults() {
        for environment in [Environment::Terminal, Environment::Wasm, Environment::WasmThreads] {
            for execution in [Execution::Manual, Execution::Runtime] {
                if environment == Environment::Wasm && execution == Execution::Runtime {
                    continue;
                }
                let catalog = catalog(Path::new("/a checkout"), selection(environment, execution), &[]).unwrap();
                catalog.validate().unwrap();
                for launch in &catalog.launches {
                    let args: Vec<_> = launch.args.iter().map(|arg| arg.to_string_lossy()).collect();
                    assert!(args.windows(2).any(|pair| pair == ["--domain", launch.visualization.slug()]));
                    assert!(args.windows(2).any(|pair| pair == ["--execution", execution.slug()]));
                    assert!(args.iter().any(|arg| arg == "--metrics-stdio"));
                    assert!(args.iter().any(|arg| arg == "--control-input"));
                    if launch.visualization == Visualization::Movement {
                        assert!(args.windows(2).any(|pair| pair == ["--jitter-ms", "500"]));
                        assert!(!args.iter().any(|arg| arg == "--services"));
                    } else if launch.visualization == Visualization::Uptime {
                        assert!(args.windows(2).any(|pair| pair == ["--jitter-ms", "800"]));
                        assert!(!args.iter().any(|arg| arg == "--clients"));
                    }
                    if environment == Environment::Terminal {
                        assert!(launch.program.to_string_lossy().ends_with("target/release/contime-wasm-example"));
                    } else if environment == Environment::Wasm {
                        assert_eq!(launch.program, "node");
                    } else {
                        assert!(args.iter().any(|arg| arg.contains("wasm32-wasip1-threads")));
                    }
                }
            }
        }
    }
    #[test]
    fn browser_chooser_preserves_shared_query_frequency() {
        let (movement, _) = configs(selection(Environment::Browser, Execution::Manual), &["--query-hz".into(), "5".into()]).unwrap();
        let browser = BrowserSettings { budget_ms: 1.0, max_steps: 1000 };
        let url = browser.url("http://localhost/", Domain::Visualizer, &browser_settings(&movement).unwrap());
        assert!(url.contains("query_hz=5"), "{url}");
    }
    #[test]
    fn shared_settings_reach_both_cases_and_reject_case_specific_or_headless_runs() {
        let selected = selection(Environment::Terminal, Execution::Runtime);
        let args = ["--workers", "2", "--query-hz", "5", "--duration", "2"].map(str::to_owned);
        let (movement, uptime) = configs(selected, &args).unwrap();
        assert_eq!((movement.workers, uptime.workers, movement.query_hz, uptime.query_hz), (2, 2, 5, 5));
        assert_eq!(movement.duration, uptime.duration);
        for flag in ["--clients", "--services", "--headless", "--events-per-second"] {
            assert!(configs(selected, &[flag.into(), "1".into()]).is_err());
        }
    }
}
