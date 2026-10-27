//! Host choices wrap the same workload settings used by the other live hosts.
use contime_wasm_example::support::Settings;

pub(super) struct Options {
    pub domain: String,
    pub execution: String,
    pub settings: Settings,
    pub metrics_stdio: bool,
    pub movement: Option<contime_wasm_example::movement::Config>,
    pub controls: Option<contime_wasm_example::input::Mode>,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, String> {
        let mut domain = "counter".to_owned();
        let mut execution = "manual".to_owned();
        let mut metrics_stdio = false;
        let mut controls = None;
        let mut common = Vec::new();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--help" | "-h" => return Ok(None),
                "--metrics-stdio" => metrics_stdio = true,
                "--control-stdin" => controls = Some(contime_wasm_example::input::Mode::Nonblocking),
                "--control-input" => {
                    controls = Some(match args.next().as_deref() {
                        Some("nonblocking") => contime_wasm_example::input::Mode::Nonblocking,
                        Some("poll") => contime_wasm_example::input::Mode::Poll,
                        _ => return Err("--control-input requires nonblocking or poll".into()),
                    })
                }
                "--domain" | "--execution" => {
                    let value = args.next().ok_or_else(|| format!("{flag} requires a value"))?;
                    if flag == "--domain" {
                        domain = value;
                    } else {
                        execution = value;
                    }
                }
                _ => common.push(flag),
            }
        }
        if !["counter", "chain-hash", "client-movement"].contains(&domain.as_str()) {
            return Err("Domain must be counter, chain-hash or client-movement".into());
        }
        if !["manual", "runtime"].contains(&execution.as_str()) {
            return Err("Execution must be manual or runtime".into());
        }
        if execution == "runtime" && !cfg!(feature = "runtime") {
            return Err("Runtime execution needs --features runtime and a thread-capable host".into());
        }
        if domain == "client-movement" {
            common.extend(["--execution".into(), execution.clone(), "--no-setup".into()]);
            let config = contime_wasm_example::movement::Config::parse(common)?.ok_or("Missing movement settings")?;
            return Ok(Some(Self { domain, execution, settings: config.settings()?, movement: Some(config), metrics_stdio, controls }));
        }
        let explicit_duration = common.iter().any(|arg| arg == "--duration");
        let Some((mut settings, _)) = Settings::parse(common, execution == "manual").map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        // WASI stdout is commonly redirected. Presentation must not add a deadline.
        if !explicit_duration {
            settings.duration = None;
        }
        settings.validate().map_err(|e| e.to_string())?;
        Ok(Some(Self { domain, execution, settings, metrics_stdio, controls, movement: None }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Option<Options>, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }
    #[test]
    fn metrics_stdio_is_host_presentation_only() {
        let options = parse(&["--metrics-stdio", "--snapshots", "8"]).unwrap().unwrap();
        assert!(options.metrics_stdio);
        assert_eq!(options.settings.snapshot_count, 8);
        assert_eq!(options.settings.duration, None);
        assert!(!parse(&[]).unwrap().unwrap().metrics_stdio);
        assert!(parse(&["--metrics-stdio", "true"]).is_err());
    }
    #[test]
    fn lives_until_stopped_unless_duration_is_explicit() {
        for args in [vec![], vec!["--headless"], vec!["--no-setup"]] {
            let options = parse(&args).unwrap().unwrap();
            assert_eq!(options.domain, "counter");
            assert_eq!(options.execution, "manual");
            assert_eq!(options.settings.duration, None);
        }
        let options = parse(&["--duration", "2"]).unwrap().unwrap();
        assert_eq!(options.settings.duration, Some(std::time::Duration::from_secs(2)));
    }
    #[test]
    fn shares_workload_settings_and_rejects_old_verifier_options() {
        let options = parse(&[
            "--snapshots",
            "8",
            "--average-snapshots-per-event",
            "2.5",
            "--max-snapshots-per-event",
            "4",
            "--events-per-second",
            "100",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(options.settings.snapshot_count, 8);
        assert_eq!(options.settings.average_targets, "2.5".parse().unwrap());
        assert_eq!(options.settings.max_targets, 4);
        for args in [
            vec!["--rounds", "8"],
            vec!["--events-per-round", "32"],
            vec!["--domain", "bad"],
            vec!["--execution", "bad"],
            vec!["--workers", "2"],
            vec!["--duration", "0"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
