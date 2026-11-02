//! Startup-only common execution settings; this teaching scenario uses a fixed delivery script.
use std::time::Duration;
#[derive(Clone, Debug)]
pub struct Config {
    pub manual: bool,
    pub workers: usize,
    pub retention_ms: u64,
    pub checkpoint_interval: u64,
    pub query_hz: u64,
    pub duration: Option<Duration>,
}
impl Default for Config {
    fn default() -> Self {
        Self { manual: false, workers: 1, retention_ms: 10000, checkpoint_interval: 128, query_hz: 20, duration: None }
    }
}
impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut config = Self::default();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            if flag == "--no-setup" {
                continue;
            }
            let value = args.next().ok_or_else(|| format!("{flag} requires a value"))?;
            if flag == "--execution" {
                config.manual = match value.as_str() {
                    "manual" => true,
                    "runtime" => false,
                    _ => return Err("Execution must be manual or runtime".into()),
                };
                continue;
            }
            let n: u64 = value.parse().map_err(|_| format!("Invalid {flag}"))?;
            match flag.as_str() {
                "--workers" => config.workers = usize::try_from(n).map_err(|_| "Worker count too large")?,
                "--query-hz" => config.query_hz = n,
                "--retention-ms" => config.retention_ms = n,
                "--checkpoint-interval" => config.checkpoint_interval = n,
                "--duration" => config.duration = Some(Duration::from_secs(n)),
                _ => return Err(format!("Unknown account option {flag}")),
            }
        }
        if !(1..=1024).contains(&config.workers)
            || (config.manual && config.workers != 1)
            || !(1..=60).contains(&config.query_hz)
            || !(1..=(u64::MAX - 1) / 1_000_000).contains(&config.retention_ms)
            || !(1..=1_000_000_000).contains(&config.checkpoint_interval)
            || config.duration.is_some_and(|d| d.is_zero() || d.as_nanos() > u128::from(u64::MAX - 1))
        {
            return Err("Invalid account execution settings".into());
        }
        Ok(config)
    }
    pub fn arguments(&self) -> Vec<String> {
        let mut args = vec!["--execution".into(), if self.manual { "manual" } else { "runtime" }.into()];
        for (key, value) in [
            ("--workers", self.workers as u64),
            ("--query-hz", self.query_hz),
            ("--retention-ms", self.retention_ms),
            ("--checkpoint-interval", self.checkpoint_interval),
        ] {
            args.extend([key.into(), value.to_string()]);
        }
        if let Some(duration) = self.duration {
            args.extend(["--duration".into(), duration.as_secs().to_string()]);
        }
        args
    }
}
