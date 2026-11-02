//! Central example selection. Existing definitions and entry points stay separate.
#[path = "../visualizer_catalog.rs"]
#[allow(dead_code)] // Shared catalogue also supplies Bevy UI labels.
mod catalog;
pub(crate) mod form;
mod graphics;
mod launch;
mod menu;
pub(crate) mod selection;
mod server;
pub(crate) mod settings_form;
mod terminal;
mod visualizer;
use selection::{Domain, Environment, Options};
use std::{
    io::{self, IsTerminal},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
const HELP: &str = "ConTime example launcher\n\nRun without arguments to select environment, execution, example and common live settings. Visualizer opens a graphical case chooser; direct client-movement and service-uptime choices bypass it.\n\n--environment native|wasm|wasm-threads|browser\n--execution runtime|manual\n--example counter|chain-hash|visualizer|client-movement|service-uptime\n--dry-run       show the launch plan\n--no-setup      skip interactive forms\n--no-open       serve browser without opening it\n--list          show supported combinations\n\nCommon settings follow -- for every environment, for example:\n-- --snapshots 8 --events-per-second 1000 --query-hz 20 --duration 2\nRuntime accepts --workers N; Manual uses one worker.\nRuns continue until stopped unless an explicit duration is selected.\nBrowser only: --budget-ms 0.01_TO_1000, --max-steps 1_TO_100000\nAliases before --: --workers N, --rate max_OR_N\n\nVisualizer graphics use an isolated Bevy example with Rust 1.95.0. Portable workload builds use Rust 1.90.0. Threaded WASI requires Wasmtime 36.x LTS\n(selected with WASMTIME); browser bindings need wasm-bindgen-cli (WASM_BINDGEN).\nwasm-bindgen checks compatibility during binding generation.\nMissing tools are reported, never installed automatically.";
const MATRIX:&str="Native: Runtime or Manual; Counter, Chain hash, Visualizer (Client movement or Service uptime)\nStandalone WASI (Node): Manual; Counter, Chain hash, Visualizer (Client movement or Service uptime)\nStandalone WASI threads (Wasmtime 36.x LTS): Manual or Runtime; Counter, Chain hash, Visualizer (Client movement or Service uptime)\nBrowser: Manual; Counter, Chain hash, Visualizer (Client movement or Service uptime)";
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(std::env::args().skip(1))?;
    if options.help || options.example_args.iter().any(|a| matches!(a.as_str(), "--help" | "-h")) {
        println!("{HELP}");
        return Ok(());
    }
    if options.list {
        println!("{MATRIX}");
        return Ok(());
    }
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal() && !options.no_setup && !options.dry_run;
    println!("ConTime / Example launcher");
    let Some(selection) = menu::selection(&options, interactive)? else {
        return Ok(());
    };
    println!("\n{} / {} / {}", selection.environment.label(), selection.execution.slug(), selection.domain.label());
    if let Some(target) = selection.environment.wasm_target() {
        println!("Target: {}", target.triple());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    options.validate(selection)?;
    let mut setup = settings_form::SettingsForm::new(selection, &options)?;
    if interactive && !options.example_args.iter().any(|a| matches!(a.as_str(), "--headless" | "--no-setup")) && !setup.run()? {
        return Ok(());
    }
    // One launcher invocation owns exactly one run and its cancellation policy.
    let cancelled = Arc::new(AtomicBool::new(false));
    if !options.dry_run {
        let stop = cancelled.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::Release))?;
    }
    if selection.domain == Domain::Visualizer {
        return visualizer::run(root, selection, &setup.arguments()?, setup.browser()?, &options, cancelled);
    }
    if selection.domain == Domain::Movement {
        let config = setup.movement()?;
        let settings = config.settings()?;
        let arguments = config.arguments();
        return match selection.environment {
            Environment::Terminal => {
                if config.headless {
                    launch::native(launch::graphical_command(root, selection, &arguments), options.dry_run, &cancelled)
                } else {
                    graphics::native(root, selection, &arguments, options.dry_run, &cancelled)
                }
            }
            Environment::Wasm | Environment::WasmThreads => {
                launch::wasi(root, selection, &settings, Some(&arguments), options.dry_run, &cancelled)
            }
            Environment::Browser => launch::browser(
                root,
                selection.domain,
                &settings,
                Some(&config),
                None,
                setup.browser()?,
                options.dry_run,
                options.no_open,
                cancelled,
            ),
        };
    }
    if selection.domain == Domain::Uptime {
        let config = setup.uptime()?;
        let mut settings = crate::support::Settings::parse(
            [
                "--snapshots".into(),
                config.services.to_string(),
                "--max-snapshots-per-event".into(),
                "1".into(),
                "--workers".into(),
                config.workers.to_string(),
                "--checkpoint-interval".into(),
                config.checkpoint_interval.to_string(),
            ],
            config.manual,
        )?
        .ok_or("Missing settings")?
        .0;
        settings.retention_ms = config.retention_ms;
        settings.headless = config.headless;
        settings.duration = config.duration;
        let arguments = config.arguments();
        return match selection.environment {
            Environment::Terminal if config.headless => {
                launch::native(launch::graphical_command(root, selection, &arguments), options.dry_run, &cancelled)
            }
            Environment::Terminal => graphics::native(root, selection, &arguments, options.dry_run, &cancelled),
            Environment::Wasm | Environment::WasmThreads => {
                launch::wasi(root, selection, &settings, Some(&arguments), options.dry_run, &cancelled)
            }
            Environment::Browser => launch::browser(
                root,
                selection.domain,
                &settings,
                None,
                Some(&config),
                setup.browser()?,
                options.dry_run,
                options.no_open,
                cancelled,
            ),
        };
    }
    let settings = setup.common()?;
    match selection.environment {
        Environment::Terminal => launch::native(launch::native_command(root, selection, &settings), options.dry_run, &cancelled),
        Environment::Wasm | Environment::WasmThreads => launch::wasi(root, selection, &settings, None, options.dry_run, &cancelled),
        Environment::Browser => {
            launch::browser(root, selection.domain, &settings, None, None, setup.browser()?, options.dry_run, options.no_open, cancelled)
        }
    }
}

/// Direct native startup uses the same isolated graphical guest as the launcher.
pub fn uptime_graphics(mut config: crate::support::uptime_config::Config) -> Result<(), Box<dyn std::error::Error>> {
    let cancelled = AtomicBool::new(false);
    let selection = selection::Selection::new(
        Environment::Terminal,
        if config.manual { selection::Execution::Manual } else { selection::Execution::Runtime },
        Domain::Uptime,
    )?;
    if !config.no_setup && io::stdin().is_terminal() && io::stdout().is_terminal() {
        let mut args = config.arguments().into_iter();
        let mut example_args = Vec::new();
        while let Some(arg) = args.next() {
            if arg == "--execution" {
                args.next();
            } else {
                example_args.push(arg);
            }
        }
        let options = Options { example_args, ..Options::default() };
        let mut setup = settings_form::SettingsForm::new(selection, &options)?;
        if !setup.run()? {
            return Ok(());
        }
        config = setup.uptime()?;
    }
    graphics::native(Path::new(env!("CARGO_MANIFEST_DIR")), selection, &config.arguments(), false, &cancelled)
}
