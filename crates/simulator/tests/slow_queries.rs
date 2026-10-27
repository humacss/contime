//! Deterministic finite-capacity host: query replies cover captured prefixes.
use contime_monitor::{Monitor, Totals};
use contime_simulator::{Maximum, MaximumConfig};
use std::time::Duration;

#[test]
fn one_worker_one_hz_overload_converges_without_runaway_query_backlogs() {
    let mut monitor = Monitor::default();
    monitor.sample(Duration::ZERO).unwrap();
    let mut controller = Maximum::new(MaximumConfig { observation_interval: Duration::from_secs(1), ..MaximumConfig::default() }).unwrap();
    let mut processed = 0u64;
    let mut active = None;
    let mut next_query = 1000;
    let mut late_queries = 0;
    let mut maximum_outstanding = 0;
    let mut last_query_duration = Duration::ZERO;
    for tick in 0..35_000 {
        let at = Duration::from_millis(tick);
        // One worker can process 100k units/s. It never processes future work.
        processed = processed.saturating_add(100).min(monitor.submitted().applications);
        if let Some(query) = active {
            let query: contime_monitor::Query = query;
            if processed >= query.submitted.applications {
                last_query_duration = monitor.complete_query(query, at).unwrap().duration;
                late_queries += usize::from(last_query_duration > Duration::from_millis(50));
                active = None;
            }
        }
        let available = controller.allowance(at, &monitor).unwrap();
        if tick < 30_000 {
            if available > 0 {
                controller.consume(available).unwrap();
                monitor.record_submission(Totals { events: available, applications: available }).unwrap();
            }
            controller.constrained();
        }
        maximum_outstanding = maximum_outstanding.max(monitor.submitted().applications - monitor.confirmed().applications);
        if active.is_none() && (tick >= next_query || (tick >= 30_000 && monitor.confirmed() != monitor.submitted())) {
            monitor.sample(at).unwrap();
            active = Some(monitor.start_query(at).unwrap());
            next_query = tick + 1000;
        }
    }
    assert!(late_queries > 0, "fixture must exercise real overload");
    assert!(maximum_outstanding < 250_000, "feedback must bound the queue");
    assert!(controller.rate() < 200_000, "controller must back off from unsustainable load");
    assert!(last_query_duration < Duration::from_millis(50));
    assert_eq!(monitor.confirmed(), monitor.submitted(), "final validation drains all accepted work");
}

#[test]
fn absent_queries_never_accumulate_unbounded_work_or_multiply_the_rate() {
    let mut monitor = Monitor::default();
    monitor.sample(Duration::ZERO).unwrap();
    let mut controller = Maximum::new(MaximumConfig { observation_interval: Duration::from_secs(1), ..MaximumConfig::default() }).unwrap();
    for tick in 0..120_000 {
        let available = controller.allowance(Duration::from_millis(tick), &monitor).unwrap();
        controller.consume(available).unwrap();
        monitor.record_submission(Totals { events: available, applications: available }).unwrap();
        controller.constrained();
    }
    assert!(controller.paused());
    assert!(monitor.submitted().applications <= 210_200);
    assert_eq!(controller.rate(), 160_000, "missing feedback is one backoff episode");
}
