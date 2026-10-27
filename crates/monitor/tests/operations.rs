//! Scenario checks paired with capacity-dependent operation benchmarks.
use contime_monitor::{Monitor, Totals};
use std::{num::NonZeroUsize, time::Duration};
#[test]
fn snapshot_rollover_compaction_and_completion_hold_at_all_benchmarked_capacities() {
    for capacity in [1, 120, 4096] {
        let mut monitor = Monitor::new(NonZeroUsize::new(capacity).unwrap());
        monitor.sample(Duration::ZERO).unwrap();
        for tick in 1..=capacity + 3 {
            monitor.record_submission(Totals { events: 1, applications: 3 }).unwrap();
            monitor.sample(Duration::from_nanos(tick as u64)).unwrap();
        }
        let at = Duration::from_nanos((capacity + 3) as u64);
        let view = monitor.snapshot(at).unwrap();
        assert_eq!(view.submitted, Totals { events: capacity as u64 + 3, applications: (capacity as u64 + 3) * 3 });
        assert_eq!(view.outstanding, view.submitted);
        assert_eq!(view.oldest_outstanding_age, Some(at));
        assert_eq!(monitor.history().len(), capacity);
        assert_eq!(monitor.history().next().unwrap().at, Duration::from_nanos(4));
        let original: Vec<_> = monitor.history().copied().collect();
        monitor.snapshot(at).unwrap();
        assert_eq!(monitor.history().copied().collect::<Vec<_>>(), original);
        // Repeated empty samples roll history without generating new cohorts.
        monitor.sample(at).unwrap();
        let query = monitor.start_query(at).unwrap();
        monitor.complete_query(query, at).unwrap();
        let final_view = monitor.sample(at).unwrap();
        assert_eq!(final_view.confirmed, view.submitted);
        assert_eq!(final_view.outstanding, Totals::default());
        assert_eq!(final_view.oldest_outstanding_age, None);
    }
}
