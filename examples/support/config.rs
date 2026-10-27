#[cfg(test)]
use super::EventRate;
use std::io::{self, IsTerminal};
#[cfg(test)]
use std::time::Duration;

pub use super::settings::Settings;
impl Settings {
    #[allow(dead_code)] // Native and manual examples select different entry helpers.
    pub fn from_env(title: &str) -> io::Result<Option<Self>> {
        Self::from_env_backend(title, false)
    }
    #[allow(dead_code)] // Native and manual examples select different entry helpers.
    pub fn from_env_manual(title: &str) -> io::Result<Option<Self>> {
        Self::from_env_backend(title, true)
    }
    fn from_env_backend(title: &str, manual: bool) -> io::Result<Option<Self>> {
        // Execution is selected by the host before common load settings are read.
        let mut args = Vec::new();
        let mut input = std::env::args().skip(1);
        while let Some(arg) = input.next() {
            if arg == "--execution" {
                input.next();
            } else {
                args.push(arg);
            }
        }
        Self::from_arguments(title, args, manual)
    }
    /// Configure an example from explicit arguments, including the shared form.
    pub fn from_arguments(title: &str, args: impl IntoIterator<Item = String>, manual: bool) -> io::Result<Option<Self>> {
        let args: Vec<String> = args.into_iter().collect();
        let Some((mut settings, no_setup)) = Self::parse(args.clone(), manual)? else {
            println!(
                "{title}\n\n--execution manual|runtime\n--snapshots N  --workers N (runtime only)\n--snapshot-skew 0..100 (0 = even, 100 = concentrated)\n--average-snapshots-per-event DECIMAL  --max-snapshots-per-event N\n--events-per-second max|N  --query-hz N  --retention-ms N\n--checkpoint-interval N  --max-pending N (numeric rates only)  --duration SECONDS\n--no-setup  --headless  --help\n\nDefaults: one worker per available logical CPU, 1,000 snapshots per worker,\nautomatically managed Maximum production, average 1.0 snapshots/event, maximum 2 (or snapshot count), 20 queries/s, 10 s retention, 128-event checkpoints.\nManual uses one worker. Headless runs default to five seconds.\nUse release builds for performance comparisons; UI and verification have their own cost."
            );
            return Ok(None);
        };
        if !no_setup && !settings.headless && io::stdin().is_terminal() && io::stdout().is_terminal() {
            use super::launcher::{
                selection::{Domain, Environment, Execution, Options, Selection},
                settings_form::SettingsForm,
            };
            let execution = if manual { Execution::Manual } else { Execution::Runtime };
            let selection = Selection::new(Environment::Terminal, execution, Domain::Counter).map_err(io::Error::other)?;
            let options = Options { example_args: args, ..Options::default() };
            let mut form = SettingsForm::new(selection, &options).map_err(io::Error::other)?;
            if !form.run_named(title)? {
                return Ok(None);
            }
            settings = form.common().map_err(io::Error::other)?;
        } else if !settings.headless && !io::stdout().is_terminal() {
            return Err(io::Error::other("a dashboard needs a terminal; use --headless --duration 5"));
        }
        settings.validate()?;
        Ok(Some(settings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str], manual: bool) -> io::Result<Option<(Settings, bool)>> {
        Settings::parse(args.iter().map(|s| s.to_string()), manual)
    }
    #[test]
    fn rejects_invalid_and_dangerously_large_settings() {
        for args in [
            vec!["--workers", "0"],
            vec!["--snapshots", "1000001"],
            vec!["--events-per-second", "-1"],
            vec!["--query-hz", "1001"],
            vec!["--duration", "0"],
            vec!["--retention-ms", "0"],
            vec!["--events-per-second", "1000", "--max-pending", "0"],
            vec!["--workers"],
            vec!["--oops", "1"],
            vec!["--duration", "18446744073709551616"],
        ] {
            assert!(parse(&args, false).is_err(), "{args:?}");
        }
    }
    #[test]
    fn manual_rejects_thread_settings_and_headless_has_a_deadline() {
        assert!(parse(&["--workers", "2"], true).is_err());
        let s = parse(&["--headless"], true).unwrap().unwrap().0;
        assert_eq!(s.workers, 1);
        assert_eq!(s.duration, Some(Duration::from_secs(5)));
    }
    #[test]
    fn old_counter_flag_and_finite_runs_still_work() {
        let s = parse(&["--counters", "7", "--workers", "2", "--duration", "1"], false).unwrap().unwrap().0;
        assert_eq!(s.snapshot_count, 7);
        assert_eq!(s.workers, 2);
        assert_eq!(s.duration, Some(Duration::from_secs(1)));
        assert!(parse(&["--help"], false).unwrap().is_none());
    }
}

#[cfg(test)]
mod snapshot_count_tests {
    use super::Settings;
    #[test]
    fn snapshot_count_option_selects_independent_snapshots() {
        for flag in ["--snapshots", "--histories", "--counters"] {
            let (settings, _) = Settings::parse([flag.to_owned(), "7".to_owned()], false).unwrap().unwrap();
            assert_eq!(settings.snapshot_count, 7);
        }
    }
}

#[cfg(test)]
mod load_default_tests {
    use super::Settings;
    #[test]
    fn worker_count_sets_the_default_workload_regardless_of_argument_order() {
        let (settings, _) = Settings::parse(["--workers", "3"].map(str::to_owned), false).unwrap().unwrap();
        assert_eq!(settings.snapshot_count, 3_000);
        assert_eq!(settings.event_rate, super::EventRate::Maximum);
        assert_eq!(settings.checkpoint_interval, 128);
        assert_eq!(settings.query_hz, 20);
        assert_eq!(settings.retention_ms, 10_000);
        assert_eq!(settings.max_pending, 60_000);
    }
    #[test]
    fn manual_defaults_are_scaled_for_its_single_worker() {
        let (settings, _) = Settings::parse([], true).unwrap().unwrap();
        assert_eq!((settings.workers, settings.snapshot_count, settings.event_rate), (1, 1_000, super::EventRate::Maximum));
    }
}

#[cfg(test)]
mod machine_default_tests {
    use super::*;
    #[test]
    fn runtime_defaults_use_the_available_logical_cpus() {
        let (settings, _) = Settings::parse([], false).unwrap().unwrap();
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get()).min(1024);
        assert_eq!(settings.workers, workers);
        assert_eq!(settings.snapshot_count, Settings::snapshot_default(workers));
        assert_eq!(settings.event_rate, EventRate::Maximum);
    }
    #[test]
    fn explicit_rate_and_workload_are_independent_of_flag_order() {
        for args in [
            ["--workers", "3", "--events-per-second", "100", "--snapshots", "7", "--max-pending", "16"],
            ["--max-pending", "16", "--snapshots", "7", "--events-per-second", "100", "--workers", "3"],
        ] {
            let (settings, _) = Settings::parse(args.map(str::to_owned), false).unwrap().unwrap();
            assert_eq!(settings.workers, 3);
            assert_eq!(settings.snapshot_count, 7);
            assert_eq!(settings.event_rate, EventRate::PerSecond(100));
            assert_eq!(settings.max_pending, 16);
            assert!(!settings.automatic_snapshots && !settings.automatic_pending);
        }
        let (settings, _) = Settings::parse(["--events-per-second", "max"].map(str::to_owned), false).unwrap().unwrap();
        assert_eq!(settings.event_rate, EventRate::Maximum);
        assert!(Settings::parse(["--events-per-second", "0"].map(str::to_owned), false).is_err());
    }
}

#[cfg(test)]
mod target_settings_tests {
    use super::*;
    #[test]
    fn averages_and_limits_are_validated_together_without_changing_the_load() {
        let parse = |args: &[&str]| Settings::parse(args.iter().map(|s| s.to_string()), false);
        let settings =
            parse(&["--snapshots", "8", "--average-snapshots-per-event", "1.1", "--max-snapshots-per-event", "4"]).unwrap().unwrap().0;
        assert_eq!(settings.average_targets, "1.1".parse().unwrap());
        assert_eq!(settings.max_targets, 4);
        for args in [
            vec!["--average-snapshots-per-event", "2.1", "--max-snapshots-per-event", "2"],
            vec!["--snapshots", "2", "--max-snapshots-per-event", "3"],
            vec!["--max-snapshots-per-event", "0"],
            vec!["--average-snapshots-per-event", "NaN"],
            vec![
                "--events-per-second",
                "1000",
                "--average-snapshots-per-event",
                "1.1",
                "--max-snapshots-per-event",
                "4",
                "--max-pending",
                "0",
            ],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        let single = parse(&["--snapshots", "1", "--max-pending", "1"]).unwrap().unwrap().0;
        assert_eq!(single.average_targets, crate::support::Average::ONE);
        assert_eq!(single.max_targets, 1);
    }
}

#[cfg(test)]
mod skew_tests {
    use super::*;
    #[test]
    fn snapshot_skew_defaults_to_even_and_accepts_only_zero_to_one_hundred() {
        let parse = |args: &[&str]| Settings::parse(args.iter().map(|s| s.to_string()), true);
        assert_eq!(parse(&[]).unwrap().unwrap().0.snapshot_skew, 0);
        for value in ["0", "50", "99", "100"] {
            assert_eq!(parse(&["--snapshot-skew", value]).unwrap().unwrap().0.snapshot_skew, value.parse::<usize>().unwrap());
        }
        for value in ["101", "-1", "NaN", "50.5"] {
            assert!(parse(&["--snapshot-skew", value]).is_err());
        }
    }
}

#[cfg(test)]
mod source_window_tests {
    #[test]
    fn a_single_source_slot_can_hold_one_multi_target_event() {
        let settings = super::Settings::parse(
            [
                "--workers",
                "1",
                "--snapshots",
                "8",
                "--events-per-second",
                "1000",
                "--average-snapshots-per-event",
                "3",
                "--max-snapshots-per-event",
                "3",
                "--max-pending",
                "1",
            ]
            .map(str::to_owned),
            true,
        )
        .unwrap()
        .unwrap()
        .0;
        assert_eq!(settings.max_pending, 1);
        assert_eq!(settings.max_targets, 3);
    }
}
