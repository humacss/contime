//! Pure selection and validation. This module starts no processes or server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Terminal,
    Wasm,
    WasmThreads,
    Browser,
}
impl Environment {
    pub fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Native",
            Self::Wasm => "WebAssembly / WASI",
            Self::WasmThreads => "WebAssembly / WASI Threads",
            Self::Browser => "Browser (WebAssembly)",
        }
    }
    pub fn wasm_target(self) -> Option<WasmTarget> {
        match self {
            Self::Wasm => Some(WasmTarget::Wasi),
            Self::WasmThreads => Some(WasmTarget::Threads),
            _ => None,
        }
    }
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "terminal" | "native" => Ok(Self::Terminal),
            "wasm" | "wasi" => Ok(Self::Wasm),
            "wasm-threads" | "wasi-threads" => Ok(Self::WasmThreads),
            "browser" => Ok(Self::Browser),
            _ => Err("Environment must be native, wasm, wasm-threads or browser".into()),
        }
    }
}
/// Standalone WASI targets have different host and thread capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WasmTarget {
    Wasi,
    Threads,
}
impl WasmTarget {
    pub fn triple(self) -> &'static str {
        match self {
            Self::Wasi => "wasm32-wasip1",
            Self::Threads => "wasm32-wasip1-threads",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Execution {
    Manual,
    Runtime,
}
impl Execution {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Runtime => "runtime",
        }
    }
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "manual" => Ok(Self::Manual),
            "runtime" => Ok(Self::Runtime),
            _ => Err("Execution must be manual or runtime".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Counter,
    Movement,
    Hash,
}
impl Domain {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Movement => "client-movement",
            Self::Hash => "chain-hash",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Counter => "Counter",
            Self::Movement => "Client movement",
            Self::Hash => "Chain hash verification",
        }
    }
    pub fn binary(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Movement => "client_movement",
            Self::Hash => "chain_hash",
        }
    }
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "counter" => Ok(Self::Counter),
            "client-movement" => Ok(Self::Movement),
            "chain-hash" => Ok(Self::Hash),
            _ => Err("Example must be counter, client-movement or chain-hash".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub environment: Environment,
    pub execution: Execution,
    pub domain: Domain,
}
impl Selection {
    pub fn new(environment: Environment, execution: Execution, domain: Domain) -> Result<Self, String> {
        if !matches!(environment, Environment::Terminal | Environment::WasmThreads) && execution == Execution::Runtime {
            return Err("Runtime execution requires Native or WebAssembly / WASI Threads".into());
        }
        Ok(Self { environment, execution, domain })
    }
}
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub environment: Option<Environment>,
    pub execution: Option<Execution>,
    pub domain: Option<Domain>,
    pub budget_ms: Option<f64>,
    pub max_steps: Option<usize>,
    pub example_args: Vec<String>,
    pub no_setup: bool,
    pub dry_run: bool,
    pub no_open: bool,
    pub help: bool,
    pub list: bool,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut result = Self::default();
        let mut args = args.into_iter();
        let mut seen = std::collections::HashSet::new();
        while let Some(flag) = args.next() {
            if flag == "--" {
                result.example_args.extend(args);
                break;
            }
            if !seen.insert(flag.clone()) {
                return Err(format!("Specify {flag} only once"));
            }
            match flag.as_str() {
                "--help" | "-h" => {
                    result.help = true;
                    continue;
                }
                "--list" => {
                    result.list = true;
                    continue;
                }
                "--no-setup" => {
                    result.no_setup = true;
                    continue;
                }
                "--dry-run" => {
                    result.dry_run = true;
                    continue;
                }
                "--no-open" => {
                    result.no_open = true;
                    continue;
                }
                "--environment" | "--execution" | "--example" | "--workers" | "--rate" | "--budget-ms" | "--max-steps" => {}
                _ => return Err(format!("Unknown launcher option {flag}; put common example settings after --")),
            }
            let value = args.next().ok_or_else(|| format!("{flag} requires a value"))?;
            let invalid = || format!("Invalid value for {flag}");
            match flag.as_str() {
                "--environment" => result.environment = Some(Environment::parse(&value)?),
                "--execution" => result.execution = Some(Execution::parse(&value)?),
                "--example" => result.domain = Some(Domain::parse(&value)?),
                "--workers" => result.example_args.extend([flag, value]),
                "--rate" => result.example_args.extend([
                    "--events-per-second".into(),
                    if value == "0" || value.eq_ignore_ascii_case("maximum") { "max".into() } else { value },
                ]),
                "--budget-ms" => result.budget_ms = Some(value.parse().map_err(|_| invalid())?),
                "--max-steps" => result.max_steps = Some(value.parse().map_err(|_| invalid())?),
                _ => unreachable!(),
            }
        }
        if result.budget_ms.is_some_and(|n| !n.is_finite() || !(0.01..=1000.0).contains(&n)) {
            return Err("Processing budget must be between 0.01 and 1000 ms".into());
        }
        if result.max_steps.is_some_and(|n| !(1..=100_000).contains(&n)) {
            return Err("Maximum processing steps must be between 1 and 100000".into());
        }
        Ok(result)
    }
    pub fn defaults(&self) -> Result<Selection, String> {
        let environment = self.environment.unwrap_or(Environment::Terminal);
        let execution = self.execution.unwrap_or(if matches!(environment, Environment::Terminal | Environment::WasmThreads) {
            Execution::Runtime
        } else {
            Execution::Manual
        });
        let selection = Selection::new(environment, execution, self.domain.unwrap_or(Domain::Counter))?;
        self.validate(selection)?;
        Ok(selection)
    }
    pub fn validate(&self, selection: Selection) -> Result<(), String> {
        Selection::new(selection.environment, selection.execution, selection.domain)?;
        if selection.environment != Environment::Browser && (self.budget_ms.is_some() || self.max_steps.is_some() || self.no_open) {
            return Err("Frame budgets and --no-open apply only to the browser".into());
        }
        if self.example_args.iter().any(|arg| arg == "--execution") {
            return Err("Choose --execution before the -- separator".into());
        }
        Ok(())
    }
    pub fn browser(&self) -> BrowserSettings {
        BrowserSettings { budget_ms: self.budget_ms.unwrap_or(1.0), max_steps: self.max_steps.unwrap_or(1000) }
    }
}
#[derive(Clone, Debug)]
pub struct BrowserSettings {
    pub budget_ms: f64,
    pub max_steps: usize,
}
impl BrowserSettings {
    pub fn movement_url(&self, base: &str, config: &crate::support::movement_config::Config) -> String {
        let mut url=format!("{base}?domain=client-movement&clients={}&input_hz={}&query_hz={}&average_latency_ms={}&jitter_ms={}&trail_ms={}&fade_ms={}&retention_ms={}&checkpoint_interval={}&max_ms={}&max_steps={}&autostart=1",
            config.clients,config.input_hz,config.query_hz,config.average_latency_ms,config.jitter_ms,config.trail_ms,config.fade_ms,config.retention_ms,config.checkpoint_interval,self.budget_ms,self.max_steps);
        url.push_str(&format!("&client_spacing={}&movement_speed={}", config.client_spacing, config.movement_speed));
        if let Some(duration) = config.duration {
            url.push_str(&format!("&duration={}", duration.as_secs()));
        }
        url
    }
    pub fn url(&self, base: &str, domain: Domain, settings: &crate::support::Settings) -> String {
        let mut url = format!(
            "{base}?domain={}&snapshots={}&rate={}&query_hz={}&retention_ms={}&checkpoint_interval={}&average_targets={}&max_targets={}&snapshot_skew={}&max_ms={}&max_steps={}&autostart=1",
            domain.slug(), settings.snapshot_count, settings.event_rate.limit().unwrap_or(0), settings.query_hz,
            settings.retention_ms, settings.checkpoint_interval, settings.average_targets,
            settings.max_targets, settings.snapshot_skew, self.budget_ms, self.max_steps
        );
        if settings.event_rate.limit().is_some() {
            url.push_str(&format!("&max_pending={}", settings.max_pending));
        }
        if let Some(duration) = settings.duration {
            url.push_str(&format!("&duration={}", duration.as_secs()));
        }
        url
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }
    #[test]
    fn browser_pending_capacity_is_forwarded_only_for_numeric_rates() {
        for rate in ["max", "100"] {
            let settings = crate::support::Settings::parse(["--events-per-second", rate, "--max-pending", "64"].map(str::to_owned), true)
                .unwrap()
                .unwrap()
                .0;
            let url = BrowserSettings { budget_ms: 1.0, max_steps: 1000 }.url("http://localhost/", Domain::Counter, &settings);
            assert_eq!(url.contains("max_pending="), rate != "max");
            if rate != "max" {
                assert!(url.contains("max_pending=64"));
            }
            assert!(url.contains("autostart=1"));
        }
    }
    #[test]
    fn capabilities_and_browser_only_limits_are_checked() {
        assert!(Selection::new(Environment::Browser, Execution::Runtime, Domain::Counter).is_err());
        for domain in [Domain::Counter, Domain::Hash, Domain::Movement] {
            for environment in [Environment::Terminal, Environment::Wasm, Environment::WasmThreads, Environment::Browser] {
                assert!(Selection::new(environment, Execution::Manual, domain).is_ok());
            }
            for environment in [Environment::Terminal, Environment::WasmThreads] {
                assert!(Selection::new(environment, Execution::Runtime, domain).is_ok());
            }
        }
        for execution in [Execution::Runtime, Execution::Manual] {
            assert!(Selection::new(Environment::WasmThreads, execution, Domain::Counter).is_ok());
        }
        for args in [vec!["--budget-ms", "NaN"], vec!["--max-steps", "0"], vec!["--rounds", "8"], vec!["--events-per-round", "32"]] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        assert!(parse(&["--environment", "wasm", "--budget-ms", "1"]).unwrap().defaults().is_err());
    }
    #[test]
    fn common_settings_are_accepted_for_every_host_and_rate_alias_is_normalized() {
        for host in ["native", "wasm", "wasm-threads", "browser"] {
            let options = parse(&["--environment", host, "--rate", "0", "--", "--snapshots", "8", "--duration", "2"]).unwrap();
            options.defaults().unwrap();
            let (settings, _) = crate::support::Settings::parse(options.example_args, true).unwrap().unwrap();
            assert_eq!(settings.snapshot_count, 8);
            assert_eq!(settings.event_rate.limit(), None);
            assert_eq!(settings.duration.unwrap().as_secs(), 2);
        }
    }
    #[test]
    fn browser_url_forwards_the_common_settings_without_an_implicit_stop() {
        let options = parse(&[
            "--environment",
            "browser",
            "--budget-ms",
            "0.5",
            "--",
            "--snapshots",
            "8",
            "--query-hz",
            "3",
            "--snapshot-skew",
            "90",
        ])
        .unwrap();
        let (settings, _) = crate::support::Settings::parse(options.example_args.clone(), true).unwrap().unwrap();
        let url = options.browser().url("http://localhost/", Domain::Counter, &settings);
        for expected in ["snapshots=8", "query_hz=3", "rate=0", "snapshot_skew=90", "max_ms=0.5", "average_targets=1.0", "autostart=1"] {
            assert!(url.contains(expected), "{expected}: {url}");
        }
        assert!(!url.contains("duration="));
        assert!(!url.contains("rounds="));
    }
}
