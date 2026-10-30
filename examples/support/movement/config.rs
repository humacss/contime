//! Startup-only movement settings; there is no adaptive throughput producer here.
use crate::support::{execution::Kind, movement_report::MAX_CLIENTS};
use std::time::Duration;
#[derive(Clone)]
pub struct Config {
    pub clients: usize,
    pub client_spacing: u64,
    pub movement_speed: u64,
    pub workers: usize,
    pub input_hz: u64,
    pub query_hz: u64,
    /// Expected one-way simulated delivery delay.
    pub average_latency_ms: u64,
    pub jitter_ms: u64,
    pub trail_ms: u64,
    pub fade_ms: u64,
    pub retention_ms: u64,
    automatic_retention: bool,
    pub checkpoint_interval: u64,
    pub duration: Option<Duration>,
    pub headless: bool,
    pub manual: bool,
    pub no_setup: bool,
}
impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, String> {
        let args: Vec<_> = args.into_iter().collect();
        let manual = Kind::select(args.clone(), false)? == Kind::Manual;
        let mut config = Self {
            clients: MAX_CLIENTS,
            client_spacing: 16,
            movement_speed: 6,
            workers: if manual { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).min(4) },
            input_hz: 1,
            query_hz: 20,
            average_latency_ms: 1000,
            jitter_ms: 1000,
            trail_ms: 5000,
            fade_ms: 5000,
            retention_ms: 10000,
            automatic_retention: true,
            checkpoint_interval: 128,
            duration: None,
            headless: false,
            manual,
            no_setup: false,
        };
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--help" | "-h" => return Ok(None),
                "--headless" => {
                    config.headless = true;
                    continue;
                }
                "--no-setup" => {
                    config.no_setup = true;
                    continue;
                }
                "--execution" => {
                    args.next().ok_or("--execution requires a value")?;
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
                "--clients" | "--snapshots" => config.clients = usize::try_from(value).map_err(|_| "Client count too large")?,
                "--client-spacing" => config.client_spacing = value,
                "--movement-speed" => config.movement_speed = value,
                "--workers" => config.workers = usize::try_from(value).map_err(|_| "Worker count too large")?,
                "--input-hz" => config.input_hz = value,
                "--query-hz" => config.query_hz = value,
                "--average-latency-ms" | "--latency-ms" => config.average_latency_ms = value,
                "--jitter-ms" => config.jitter_ms = value,
                "--trail-ms" => config.trail_ms = value,
                "--fade-ms" => config.fade_ms = value,
                "--retention-ms" => {
                    config.retention_ms = value;
                    config.automatic_retention = false;
                }
                "--checkpoint-interval" => config.checkpoint_interval = value,
                "--duration" => config.duration = Some(Duration::from_secs(value)),
                _ => return Err(format!("Unknown movement option {flag}")),
            }
        }
        config.refresh_retention()?;
        config.validate()?;
        Ok(Some(config))
    }
    pub fn settings(&self) -> Result<crate::support::Settings, String> {
        let mut args = vec![
            "--snapshots".into(),
            self.clients.to_string(),
            "--workers".into(),
            self.workers.to_string(),
            "--retention-ms".into(),
            self.retention_ms.to_string(),
            "--checkpoint-interval".into(),
            self.checkpoint_interval.to_string(),
        ];
        if let Some(duration) = self.duration {
            args.extend(["--duration".into(), duration.as_secs().to_string()]);
        }
        if self.headless {
            args.push("--headless".into());
        }
        let mut settings =
            crate::support::Settings::parse(args, self.manual).map_err(|e| e.to_string())?.ok_or("Missing execution settings")?.0;
        settings.duration = self.duration;
        Ok(settings)
    }
    /// Effective jitter stays symmetric even when the requested variation is large.
    pub fn jitter_radius_ms(&self) -> u64 {
        self.average_latency_ms.min(self.jitter_ms)
    }
    pub fn max_latency_ms(&self) -> u64 {
        self.average_latency_ms.saturating_add(self.jitter_radius_ms())
    }
    pub fn drain_timeout(&self) -> Duration {
        Duration::from_millis(self.max_latency_ms()).saturating_add(Duration::from_secs(15))
    }
    fn required_retention(&self) -> Result<u64, String> {
        self.average_latency_ms
            .checked_add(self.jitter_radius_ms())
            .and_then(|delay| delay.checked_add(self.trail_ms))
            .and_then(|window| window.checked_add(501))
            .ok_or_else(|| "Latency and trail settings exceed the supported time range".into())
    }
    fn refresh_retention(&mut self) -> Result<(), String> {
        if self.automatic_retention {
            self.retention_ms = 10000.max(self.required_retention()?);
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_CLIENTS).contains(&self.clients)
            || !(1..=1024).contains(&self.workers)
            || !(1..=60).contains(&self.input_hz)
            || !(1..=60).contains(&self.query_hz)
        {
            return Err("Use 1–5 clients, 1–1,024 workers and 1–60 inputs/queries per second".into());
        }
        if self.client_spacing > 1_000_000 || self.movement_speed > 1_000_000 {
            return Err("Client spacing and movement speed must be whole numbers between 0 and 1,000,000".into());
        }
        if self.manual && self.workers != 1 {
            return Err("Manual uses one worker".into());
        }
        if !(200..=5000).contains(&self.trail_ms)
            || !(100..=5000).contains(&self.fade_ms)
            || !(1..=86400000).contains(&self.retention_ms)
            || !(1..=1000000000).contains(&self.checkpoint_interval)
        {
            return Err("Trail: 200–5,000 ms; fade: 100–5,000 ms; retention: 1–86,400,000 ms; checkpoint spacing: 1–1,000,000,000".into());
        }
        if self.required_retention()? > self.retention_ms {
            return Err("Retention must exceed maximum input delay + trail duration + 500 ms".into());
        }
        crate::support::Settings::validate_duration(self.duration).map_err(|e| e.to_string())
    }
    /// Preserve automatic retention while carrying a parsed configuration into an editor.
    pub fn arguments(&self) -> Vec<String> {
        let mut args = Vec::new();
        for (flag, value) in [
            ("--clients", self.clients as u64),
            ("--client-spacing", self.client_spacing),
            ("--movement-speed", self.movement_speed),
            ("--workers", self.workers as u64),
            ("--input-hz", self.input_hz),
            ("--query-hz", self.query_hz),
            ("--average-latency-ms", self.average_latency_ms),
            ("--jitter-ms", self.jitter_ms),
            ("--trail-ms", self.trail_ms),
            ("--fade-ms", self.fade_ms),
            ("--checkpoint-interval", self.checkpoint_interval),
        ] {
            args.extend([flag.into(), value.to_string()]);
        }
        if !self.automatic_retention {
            args.extend(["--retention-ms".into(), self.retention_ms.to_string()]);
        }
        if let Some(duration) = self.duration {
            args.extend(["--duration".into(), duration.as_secs().to_string()]);
        }
        if self.headless {
            args.push("--headless".into());
        }
        args
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_average_latency_expands_retention_unless_it_was_explicit() {
        let parse = |args: &[&str]| Config::parse(args.iter().map(|s| s.to_string()));
        for latency in ["50", "1000", "30000", "300000"] {
            let config = parse(&["--average-latency-ms", latency]).unwrap().unwrap();
            assert!(config.retention_ms > latency.parse::<u64>().unwrap() + config.trail_ms + 500);
        }
        assert!(parse(&["--average-latency-ms", "30000", "--retention-ms", "10000"]).is_err());
        assert!(parse(&["--average-latency-ms", "18446744073709551615"]).is_err());
    }
    #[test]
    fn settings_keep_late_inputs_and_trails_inside_retention() {
        let parse = |args: &[&str]| Config::parse(args.iter().map(|s| s.to_string()));
        let c = parse(&[]).unwrap().unwrap();
        assert_eq!(c.input_hz, 1);
        assert!(parse(&["--retention-ms", "1000"]).is_err());
        assert!(parse(&["--execution", "manual", "--workers", "2"]).is_err());
        assert!(parse(&["--clients", "1", "--latency-ms", "0", "--jitter-ms", "0", "--duration", "315360000"]).is_ok());
    }
    #[test]
    fn spacing_and_speed_defaults_overrides_and_bounds_round_trip() {
        let parse = |args: &[&str]| Config::parse(args.iter().map(|s| s.to_string()));
        let default = parse(&[]).unwrap().unwrap();
        assert_eq!(default.clients, 5);
        assert_eq!((default.client_spacing, default.movement_speed), (16, 6));
        let custom = parse(&["--client-spacing", "9", "--movement-speed", "10"]).unwrap().unwrap();
        let round = Config::parse(custom.arguments()).unwrap().unwrap();
        assert_eq!((round.client_spacing, round.movement_speed), (9, 10));
        for flag in ["--client-spacing", "--movement-speed"] {
            for value in ["0", "1000000"] {
                assert!(parse(&[flag, value]).is_ok());
            }
            for value in ["-1", "1.5", "1000001", "NaN"] {
                assert!(parse(&[flag, value]).is_err());
            }
        }
    }
}

#[cfg(test)]
mod client_limit_tests {
    use super::*;
    #[test]
    fn movement_accepts_one_to_five_clients_and_rejects_larger_runs() {
        for clients in [1, 5] {
            assert!(Config::parse(["--clients".to_owned(), clients.to_string()]).is_ok());
        }
        for clients in [0, 6, 10000] {
            assert!(Config::parse(["--clients".to_owned(), clients.to_string()]).is_err());
        }
    }
}
