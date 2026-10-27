//! Compose the counter with the shared simulator/monitor Runtime host or Manual host.
#[path = "../../cases/counter/monitored/generation.rs"]
mod generation;
#[path = "../../cases/counter/model.rs"]
mod model;
#[path = "../../cases/counter/setup.rs"]
mod setup;
#[path = "../../support/mod.rs"]
mod support;
use definition::Time;
use monitored::Options;
use support::{definition, monitored};
#[cfg(test)]
#[path = "../../cases/counter/runtime_tests.rs"]
mod runtime_tests;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    monitored::run_configured::<setup::CounterExample, _, _>(false, generation::events, monitored::Delivery::Forward)
}

#[cfg(test)]
mod monitored_tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn startup_settings_run_counter_simulator_with_monitor_feedback() {
        for rate in ["max", "2000"] {
            let (settings, _) = support::Settings::parse(
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
                    "--headless",
                    "--duration",
                    "1",
                ]
                .map(str::to_owned),
                false,
            )
            .unwrap()
            .unwrap();
            let mut options = Options::from_settings(&settings);
            options.duration = Some(Duration::from_millis(100));
            let summary =
                monitored::run_with_options::<setup::CounterExample, _, _>(options, generation::events, monitored::Delivery::Forward)
                    .unwrap();
            assert!(summary.events > 0);
            assert_eq!(summary.completed, summary.events);
            assert!(summary.applications > summary.events);
            assert!(summary.applications <= summary.events * 3);
        }
    }
}

#[cfg(all(test, feature = "manual"))]
#[path = "../../cases/counter/monitored/manual_tests.rs"]
mod manual_monitored_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn counter_uses_shared_host_in_fixed_and_maximum_modes() {
        for (workers, targets, maximum) in [(1, 1, false), (2, 3, false), (1, 1, true), (2, 3, true)] {
            let options = Options {
                workers,
                snapshots: 8,
                targets,
                rate: 2_000,
                maximum,
                duration: Some(Duration::from_millis(100)),
                headless: true,
                ..Options::default()
            };
            let summary =
                monitored::run_with_options::<setup::CounterExample, _, _>(options, generation::events, monitored::Delivery::Forward)
                    .unwrap();
            assert!(summary.events > 0);
            assert_eq!(summary.events, summary.completed);
            assert_eq!(summary.applications, summary.events * targets as u64);
        }
    }
}
