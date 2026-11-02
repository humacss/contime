//! Startup-only settings, deliberately independent of runtime, macros and terminal UI.
use std::time::Duration;
/// Bound simulated network memory independently of the selected latency.
pub const MAX_PENDING_CHANGES: usize = 8192;
/// Maximum health-change ticks generated in one host turn.
pub const CATCH_UP_TICKS: u64 = 64;
/// Visible future portion of the service input timeline.
pub const LOOK_AHEAD_MS: u64 = 3500;
#[derive(Clone, Debug)]
pub struct Config {
    pub services: usize,
    pub workers: usize,
    pub manual: bool,
    pub retention_ms: u64,
    pub checkpoint_interval: u64,
    pub duration: Option<Duration>,
    pub headless: bool,
    pub no_setup: bool,
    pub cadence_ms: u64,
    pub latency_ms: u64,
    pub jitter_ms: u64,
    pub min_delay_ms: u64,
    pub query_hz: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            services: 5,
            workers: 1,
            manual: false,
            retention_ms: 10000,
            checkpoint_interval: 128,
            duration: None,
            headless: false,
            no_setup: false,
            cadence_ms: 1000,
            latency_ms: 800,
            jitter_ms: 800,
            min_delay_ms: 250,
            query_hz: 20,
        }
    }
}
impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, String> {
        let mut c = Self::default();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--help" | "-h" => return Ok(None),
                "--headless" => {
                    c.headless = true;
                    continue;
                }
                "--no-setup" => {
                    c.no_setup = true;
                    continue;
                }
                "--execution" => {
                    c.manual = match args.next().as_deref() {
                        Some("manual") => true,
                        Some("runtime") => false,
                        _ => return Err("Execution must be runtime or manual".into()),
                    };
                    continue;
                }
                _ => {}
            }
            let value: u64 = args
                .next()
                .ok_or_else(|| format!("{flag} requires a value"))?
                .parse()
                .map_err(|_| format!("{flag} requires a whole number"))?;
            match flag.as_str() {
                "--services" | "--snapshots" => c.services = usize::try_from(value).map_err(|_| "Service count too large")?,
                "--workers" => c.workers = usize::try_from(value).map_err(|_| "Worker count too large")?,
                "--retention-ms" => c.retention_ms = value,
                "--checkpoint-interval" => c.checkpoint_interval = value,
                "--duration" => c.duration = Some(Duration::from_secs(value)),
                "--latency-ms" | "--average-latency-ms" => c.latency_ms = value,
                "--cadence-ms" => c.cadence_ms = value,
                "--jitter-ms" => c.jitter_ms = value,
                "--min-delay-ms" => c.min_delay_ms = value,
                "--query-hz" => c.query_hz = value,
                _ => return Err(format!("Unknown uptime option {flag}")),
            }
        }
        c.validate()?;
        Ok(Some(c))
    }
    pub fn max_latency_ms(&self) -> u64 {
        self.latency_ms.saturating_add(self.jitter_ms.min(self.latency_ms)).max(self.min_delay_ms)
    }
    pub fn drain_timeout(&self) -> Duration {
        // Simulated delays are flushed on stop; only backend work remains.
        Duration::from_secs(15)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=16).contains(&self.services)
            || !(1..=1024).contains(&self.workers)
            || (self.manual && self.workers != 1)
            || !(100..=60000).contains(&self.cadence_ms)
            || !(1..=60).contains(&self.query_hz)
            || !(1..=(u64::MAX - 1) / 1_000_000).contains(&self.retention_ms)
            || !(1..=1000000000).contains(&self.checkpoint_interval)
            || self.max_latency_ms() > (u64::MAX - 1) / 1_000_000
        {
            return Err("Use 1–16 services, 1–1024 workers (one for Manual), 1–60 queries/s, retention within the application clock range, bounded latency and positive checkpoint spacing".into());
        }
        // Include future previews, one boundary tick and a full catch-up turn before delivery.
        // A finite run may stop before its configured network delay elapses.
        let horizon_ms = self.max_latency_ms().saturating_add(LOOK_AHEAD_MS);
        let horizon_ms = self.duration.map_or(horizon_ms, |duration| horizon_ms.min(duration.as_millis().min(u128::from(u64::MAX)) as u64));
        let pending = (horizon_ms.div_ceil(self.cadence_ms) + 1 + CATCH_UP_TICKS).saturating_mul(self.services as u64);
        if pending > MAX_PENDING_CHANGES as u64 {
            return Err(format!("Service count, cadence and latency need more than {MAX_PENDING_CHANGES} pending changes; reduce services or latency, increase cadence, or use a shorter duration"));
        }
        if self.headless && self.duration.is_none() {
            return Err("Headless runs require --duration SECONDS".into());
        }
        if self.duration.is_some_and(|d| d.is_zero() || d.as_nanos() > u128::from(u64::MAX - 1)) {
            return Err("Duration exceeds application clock range or is zero".into());
        }
        Ok(())
    }
    pub fn arguments(&self) -> Vec<String> {
        let mut args = Vec::new();
        for (flag, value) in [
            ("--services", self.services as u64),
            ("--workers", self.workers as u64),
            ("--retention-ms", self.retention_ms),
            ("--checkpoint-interval", self.checkpoint_interval),
            ("--cadence-ms", self.cadence_ms),
            ("--latency-ms", self.latency_ms),
            ("--jitter-ms", self.jitter_ms),
            ("--min-delay-ms", self.min_delay_ms),
            ("--query-hz", self.query_hz),
        ] {
            args.extend([flag.into(), value.to_string()]);
        }
        args.extend(["--execution".into(), if self.manual { "manual" } else { "runtime" }.into()]);
        if let Some(d) = self.duration {
            args.extend(["--duration".into(), d.as_secs().to_string()]);
        }
        if self.headless {
            args.push("--headless".into());
        }
        if self.no_setup {
            args.push("--no-setup".into());
        }
        args
    }
}
