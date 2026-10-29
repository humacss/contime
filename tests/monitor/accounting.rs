use contime::monitor::{Monitor, Totals};
use std::time::Duration;

#[test]
fn completing_a_captured_prefix_preserves_newer_outstanding_work() {
    let mut monitor = Monitor::default();
    monitor.sample(Duration::ZERO).unwrap();
    monitor.record_submission(Totals { events: 2, applications: 4 }).unwrap();
    let prefix = monitor.start_query(Duration::from_millis(10)).unwrap();
    monitor.record_submission(Totals { events: 1, applications: 3 }).unwrap();
    monitor.complete_query(prefix, Duration::from_millis(20)).unwrap();
    let first = monitor.sample(Duration::from_millis(20)).unwrap();
    assert_eq!(first.confirmed, Totals { events: 2, applications: 4 });
    assert_eq!(first.outstanding, Totals { events: 1, applications: 3 });
    let remaining = monitor.start_query(Duration::from_millis(30)).unwrap();
    monitor.complete_query(remaining, Duration::from_millis(40)).unwrap();
    let settled = monitor.sample(Duration::from_millis(40)).unwrap();
    assert_eq!(settled.confirmed, settled.submitted);
    assert_eq!(settled.outstanding, Totals::default());
    assert_eq!(monitor.history().len(), 3);
}
