//! The same counter callbacks and host, driven through Manual instead of Runtime.
use super::*;
use std::time::Duration;
#[test]
fn manual_counter_confirms_maximum_and_fixed_load_with_short_retention() {
    for (rate, maximum, query_hz) in [(2_000, false, 20), (1_000_000, false, 1), (100_000, true, 1)] {
        let options = Options {
            workers: 1,
            snapshots: 8,
            targets: 3,
            average_scaled: Some(1500),
            snapshot_skew: 90,
            rate,
            maximum,
            query_hz,
            retention_ms: 20,
            checkpoint_interval: 8,
            duration: Some(Duration::from_millis(250)),
            headless: true,
            ..Options::default()
        };
        let summary =
            monitored::run_manual_with_options::<setup::CounterExample, _, _>(options, generation::events, monitored::Delivery::Forward)
                .unwrap();
        assert!(summary.events > 0);
        assert_eq!(summary.events, summary.completed);
        assert!(summary.applications > summary.events);
        assert!(summary.applications <= summary.events * 3);
        assert!(summary.collections > 0);
        assert_eq!(summary.queries, 1); // Query frequency never gates counter production.
    }
}
#[test]
fn manual_counter_rejects_multiple_workers() {
    let options = Options { workers: 2, headless: true, duration: Some(Duration::from_millis(10)), ..Options::default() };
    let error =
        monitored::run_manual_with_options::<setup::CounterExample, _, _>(options, generation::events, monitored::Delivery::Forward)
            .unwrap_err();
    assert!(error.to_string().contains("one worker"));
}
