//! Shared startup-only workload settings for native and portable examples.
use std::time::Duration;
#[derive(Clone)]
pub struct Options {
    pub workers: usize,
    pub snapshots: usize,
    pub targets: usize,
    pub rate: u64,
    pub maximum: bool,
    pub duration: Option<Duration>,
    pub headless: bool,
    pub retention_ms: u64,
    pub checkpoint_interval: u64,
    pub query_hz: usize,
    pub average_scaled: Option<u64>,
    pub snapshot_skew: usize,
    pub source_limit: Option<u64>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            workers: 1,
            snapshots: 1_000,
            targets: 1,
            rate: 100_000,
            maximum: true,
            duration: None,
            headless: false,
            retention_ms: 10_000,
            checkpoint_interval: 128,
            query_hz: 20,
            average_scaled: None,
            snapshot_skew: 0,
            source_limit: None,
        }
    }
}
impl Options {
    /// Adapt the common startup form without changing its selected settings.
    pub fn from_settings(settings: &super::super::Settings) -> Self {
        Self {
            workers: settings.workers,
            snapshots: settings.snapshot_count,
            targets: settings.max_targets,
            rate: settings.event_rate.limit().unwrap_or(100_000) as u64,
            maximum: settings.event_rate == super::super::EventRate::Maximum,
            duration: settings.duration,
            headless: settings.headless,
            retention_ms: settings.retention_ms,
            checkpoint_interval: settings.checkpoint_interval as u64,
            query_hz: settings.query_hz,
            average_scaled: Some(settings.average_targets.scaled()),
            snapshot_skew: settings.snapshot_skew,
            source_limit: settings.event_rate.limit().map(|_| settings.max_pending as u64),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        super::super::Settings::validate_duration(self.duration).map_err(|e| e.to_string())?;
        if !(1..=1024).contains(&self.workers)
            || !(1..=1_000_000).contains(&self.snapshots)
            || self.targets == 0
            || self.targets > self.snapshots
            || self.retention_ms == 0
            || self.retention_ms > 86_400_000
            || self.checkpoint_interval == 0
            || self.checkpoint_interval > 1_000_000_000
            || !(1..=1_000).contains(&self.query_hz)
            || self.average_scaled.is_some_and(|average| average < 1_000 || average > self.targets as u64 * 1_000)
            || self.snapshot_skew > 100
            || self.source_limit == Some(0)
            || !(1..=1_000_000_000).contains(&self.rate)
        {
            return Err(
                "Use workers 1–1024, snapshots 1–1,000,000, targets 1–snapshots, snapshot-skew 0–100, events/s 1–1,000,000,000".into()
            );
        }
        if self.headless && self.duration.is_none() {
            return Err("Headless runs require --duration SECONDS".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durations_have_no_arbitrary_upper_limit() {
        for seconds in [3601, 86401, 7 * 86400, 10 * 365 * 86400] {
            let (settings, _) =
                super::super::super::Settings::parse(["--duration".to_owned(), seconds.to_string()], false).unwrap().unwrap();
            let options = Options::from_settings(&settings);
            options.validate().unwrap();
            assert_eq!(options.duration, Some(Duration::from_secs(seconds)));
        }
        let (settings, _) = super::super::super::Settings::parse([], false).unwrap().unwrap();
        assert!(settings.duration.is_none());
        Options::from_settings(&settings).validate().unwrap();
        assert!(Options { duration: Some(Duration::ZERO), ..Options::default() }.validate().is_err());
        assert!(super::super::super::Settings::parse(["--duration", "0"].map(str::to_owned), false).is_err());
    }
    #[test]
    fn common_settings_preserve_load_retention_and_validation_options() {
        for rate in ["max", "2000"] {
            let (settings, _) = super::super::super::Settings::parse(
                [
                    "--workers",
                    "2",
                    "--snapshots",
                    "8",
                    "--average-snapshots-per-event",
                    "1.5",
                    "--max-snapshots-per-event",
                    "3",
                    "--snapshot-skew",
                    "90",
                    "--events-per-second",
                    rate,
                    "--query-hz",
                    "7",
                    "--retention-ms",
                    "1000",
                    "--checkpoint-interval",
                    "16",
                    "--max-pending",
                    "64",
                    "--headless",
                    "--duration",
                    "1",
                ]
                .map(str::to_owned),
                false,
            )
            .unwrap()
            .unwrap();
            let options = Options::from_settings(&settings);
            options.validate().unwrap();
            assert_eq!(options.workers, 2);
            assert_eq!(options.snapshots, 8);
            assert_eq!(options.targets, 3);
            assert_eq!(options.average_scaled, Some(1500));
            assert_eq!(options.snapshot_skew, 90);
            assert_eq!(options.query_hz, 7);
            assert_eq!(options.retention_ms, 1000);
            assert_eq!(options.checkpoint_interval, 16);
            assert_eq!(options.duration, Some(Duration::from_secs(1)));
            assert!(options.headless);
            assert_eq!(options.maximum, rate == "max");
            assert_eq!(options.source_limit, if rate == "max" { None } else { Some(64) });
            if rate != "max" {
                assert_eq!(options.rate, 2000);
            }
        }
    }
}
