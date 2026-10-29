use contime::{
    monitor::{Monitor, Totals},
    simulator::{Maximum, MaximumConfig, WorkUnit},
};
use std::time::Duration;

#[test]
fn adaptive_admission_uses_public_monitor_progress_and_pauses_for_a_slow_query() {
    let mut monitor = Monitor::default();
    monitor.sample(Duration::ZERO).unwrap();
    let initial = 1_000;
    let mut maximum = Maximum::new(MaximumConfig { initial_rate: initial, unit: WorkUnit::Events, ..MaximumConfig::default() }).unwrap();
    assert!(maximum.allowance(Duration::ZERO, &monitor).unwrap() >= 1);
    maximum.consume(1).unwrap();
    monitor.record_submission(Totals { events: 1, applications: 2 }).unwrap();
    maximum.constrained();
    let query = monitor.start_query(Duration::ZERO).unwrap();
    assert_eq!(maximum.allowance(Duration::from_secs(1), &monitor).unwrap(), 0);
    assert!(maximum.paused());
    monitor.complete_query(query, Duration::from_secs(1)).unwrap();
    maximum.allowance(Duration::from_secs(1), &monitor).unwrap();
    assert!(maximum.rate() <= initial);
    assert_eq!(monitor.confirmed(), Totals { events: 1, applications: 2 });
}
