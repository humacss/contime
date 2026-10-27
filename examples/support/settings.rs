//! Shared live-load configuration and CLI validation, independent of host UI.
use crate::support::{Average, EventRate};
use std::{io, time::Duration};
/// Startup settings for the live examples, independent of application state.
#[derive(Clone, Debug)]
pub struct Settings {
    pub snapshot_count: usize,
    pub average_targets: Average,
    pub max_targets: usize,
    pub snapshot_skew: usize,
    pub workers: usize,
    pub event_rate: EventRate,
    pub query_hz: usize,
    pub retention_ms: u64,
    pub checkpoint_interval: usize,
    /// Numeric-rate outstanding allowance. Maximum manages its own window.
    pub max_pending: usize,
    // Remember which workload values follow worker changes in the startup form.
    pub automatic_snapshots: bool,
    pub automatic_pending: bool,
    pub duration: Option<Duration>,
    pub headless: bool,
}
impl Settings {
    pub fn parse(args: impl IntoIterator<Item = String>, manual: bool) -> io::Result<Option<(Self, bool)>> {
        let workers = if manual { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).min(1024) };
        let mut settings = Self {
            snapshot_count: Self::snapshot_default(workers),
            average_targets: Average::ONE,
            max_targets: 2,
            snapshot_skew: 0,
            workers,
            event_rate: EventRate::Maximum,
            query_hz: 20,
            retention_ms: 10_000,
            checkpoint_interval: 128,
            max_pending: Self::pending_default(workers, EventRate::Maximum),
            automatic_snapshots: true,
            automatic_pending: true,
            duration: None,
            headless: false,
        };
        let mut explicit_targets = false;
        let mut explicit_snapshots = false;
        let mut explicit_pending = false;
        let mut no_setup = false;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--help" | "-h" => return Ok(None),
                "--no-setup" => {
                    no_setup = true;
                    continue;
                }
                "--headless" => {
                    settings.headless = true;
                    continue;
                }
                _ => {}
            }
            let value = args.next().ok_or_else(|| io::Error::other(format!("{flag} requires a value")))?;
            if flag == "--events-per-second" && value == "max" {
                settings.event_rate = EventRate::Maximum;
                continue;
            }
            if flag == "--average-snapshots-per-event" {
                settings.average_targets = value.parse().map_err(io::Error::other)?;
                continue;
            }
            let value: u64 = value.parse().map_err(|_| io::Error::other(format!("{flag} requires a positive integer")))?;
            let size = usize::try_from(value).map_err(|_| io::Error::other("setting exceeds this platform's integer range"))?;
            match flag.as_str() {
                "--snapshots" | "--histories" | "--counters" => {
                    settings.snapshot_count = size;
                    explicit_snapshots = true;
                }
                "--max-snapshots-per-event" => {
                    settings.max_targets = size;
                    explicit_targets = true;
                }
                "--snapshot-skew" => settings.snapshot_skew = size,
                "--workers" => settings.workers = size,
                "--events-per-second" => settings.event_rate = EventRate::PerSecond(size),
                "--query-hz" => settings.query_hz = size,
                "--retention-ms" => settings.retention_ms = value,
                "--checkpoint-interval" => settings.checkpoint_interval = size,
                "--max-pending" => {
                    settings.max_pending = size;
                    explicit_pending = true;
                }
                "--duration" => settings.duration = Some(Duration::from_secs(value)),
                _ => return Err(io::Error::other(format!("unknown option: {flag}"))),
            }
        }
        if !explicit_snapshots {
            settings.snapshot_count = Self::snapshot_default(settings.workers);
        }
        if !explicit_pending {
            settings.max_pending = Self::pending_default(settings.workers, settings.event_rate);
        }
        if !explicit_targets {
            settings.max_targets = 2.min(settings.snapshot_count);
        }
        settings.automatic_snapshots = !explicit_snapshots;
        settings.automatic_pending = !explicit_pending;
        if manual && settings.workers != 1 {
            return Err(io::Error::other("Manual processes one worker on the calling thread; --workers must be 1"));
        }
        if settings.headless && settings.duration.is_none() {
            settings.duration = Some(Duration::from_secs(5));
        }
        settings.validate()?;
        Ok(Some((settings, no_setup)))
    }
    pub fn snapshot_default(workers: usize) -> usize {
        workers.saturating_mul(1000).min(1_000_000)
    }
    /// Approximately two seconds of reference load, with example-only memory safeguards.
    pub fn pending_default(workers: usize, rate: EventRate) -> usize {
        rate.limit().unwrap_or_else(|| workers.saturating_mul(10_000)).saturating_mul(2).clamp(4096, 1_000_000)
    }
    /// An omitted duration runs until stopped; explicit durations have no upper cap.
    pub fn validate_duration(duration: Option<Duration>) -> io::Result<()> {
        if duration.is_some_and(|d| d.is_zero()) {
            return Err(io::Error::other("duration must be positive"));
        }
        Ok(())
    }
    pub fn validate(&self) -> io::Result<()> {
        for (name, value, limit) in [
            ("snapshots", self.snapshot_count, 1_000_000),
            ("workers", self.workers, 1024),
            ("query-hz", self.query_hz, 1000),
            ("checkpoint-interval", self.checkpoint_interval, 1_000_000_000),
        ] {
            if value == 0 || value > limit {
                return Err(io::Error::other(format!("{name} must be between 1 and {limit}")));
            }
        }
        if self.snapshot_skew > 100 {
            return Err(io::Error::other("snapshot-skew must be a whole number between 0 and 100"));
        }
        if self.max_targets == 0 || self.max_targets > self.snapshot_count {
            return Err(io::Error::other("--max-snapshots-per-event must be between 1 and the snapshot count"));
        }
        if self.average_targets.scaled() > self.max_targets as u64 * 1000 {
            return Err(io::Error::other("--average-snapshots-per-event cannot exceed --max-snapshots-per-event"));
        }
        if self.event_rate != EventRate::Maximum && (self.max_pending == 0 || self.max_pending > 10_000_000) {
            return Err(io::Error::other("max-pending must be between 1 and 10000000"));
        }
        if self.event_rate.limit().is_some_and(|n| n == 0 || n > 1_000_000_000) {
            return Err(io::Error::other("events-per-second must be max or a whole number between 1 and 1000000000"));
        }
        if self.retention_ms == 0 || self.retention_ms > 86_400_000 {
            return Err(io::Error::other("retention-ms must be between 1 and 86400000"));
        }
        Self::validate_duration(self.duration)?;
        Ok(())
    }
}
