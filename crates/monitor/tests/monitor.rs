use contime_monitor::{Error, Monitor, Query, Rates, Totals};
use std::{num::NonZeroUsize, time::Duration};
fn t(events: u64, applications: u64) -> Totals {
    Totals { events, applications }
}
fn s(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[test]
fn query_confirms_its_prefix_without_newer_fanout() {
    let mut monitor = Monitor::default();
    monitor.sample(s(0)).unwrap();
    monitor.record_submission(t(2, 6)).unwrap();
    let query = monitor.start_query(s(1)).unwrap();
    monitor.record_submission(t(3, 9)).unwrap();
    let observation = monitor.complete_query(query, s(2)).unwrap();
    assert_eq!(observation.duration, s(1));
    let snapshot = monitor.sample(s(2)).unwrap();
    assert_eq!(snapshot.submitted, t(5, 15));
    assert_eq!(snapshot.confirmed, t(2, 6));
    assert_eq!(snapshot.outstanding, t(3, 9));
    assert_eq!(snapshot.recent_submitted.events_per_second, 2.5);
    assert_eq!(snapshot.recent_confirmed.applications_per_second, 3.0);
    assert_eq!(snapshot.active_query, None);
    assert_eq!(snapshot.last_query, Some(observation));
}

#[test]
fn rates_use_sample_intervals_and_zero_time_is_finite() {
    let mut monitor = Monitor::default();
    monitor.record_submission(t(4, 12)).unwrap();
    assert_eq!(monitor.sample(s(3)).unwrap().recent_submitted, Rates::default());
    monitor.record_submission(t(2, 8)).unwrap();
    let zero = monitor.snapshot(s(3)).unwrap();
    assert_eq!(zero.recent_submitted, Rates::default());
    let sample = monitor.sample(s(5)).unwrap();
    assert_eq!(sample.recent_submitted.events_per_second, 1.0);
    assert_eq!(sample.average_submitted.applications_per_second, 4.0);
    assert_eq!(monitor.snapshot(s(7)).unwrap().recent_submitted, Rates::default());
}

#[test]
fn failed_operations_leave_observable_state_unchanged() {
    let mut monitor = Monitor::default();
    monitor.sample(s(2)).unwrap();
    monitor.record_submission(t(1, 3)).unwrap();
    let query = monitor.start_query(s(3)).unwrap();
    let before = monitor.snapshot(s(4)).unwrap();
    assert_eq!(monitor.start_query(s(4)), Err(Error::QueryAlreadyActive));
    assert_eq!(monitor.sample(s(1)), Err(Error::ClockReversed));
    assert_eq!(monitor.snapshot(s(1)), Err(Error::ClockReversed));
    assert_eq!(monitor.complete_query(query, s(2)), Err(Error::ClockReversed));
    for fake in [Query { id: query.id + 1, ..query }, Query { submitted: t(9, 9), ..query }, Query { started: s(8), ..query }] {
        assert_eq!(monitor.complete_query(fake, s(4)), Err(Error::InvalidQuery));
    }
    assert_eq!(monitor.snapshot(s(4)).unwrap(), before);
    assert_eq!(monitor.history().count(), 1);
    monitor.complete_query(query, s(4)).unwrap();
    assert_eq!(monitor.complete_query(query, s(5)), Err(Error::InvalidQuery));
}

#[test]
fn overflow_is_atomic_across_both_counters() {
    let mut monitor = Monitor::default();
    monitor.record_submission(t(2, u64::MAX)).unwrap();
    assert_eq!(monitor.record_submission(t(1, 1)), Err(Error::CounterOverflow));
    assert_eq!(monitor.submitted(), t(2, u64::MAX));
    assert_eq!(monitor.record_submission(t(u64::MAX, 0)), Err(Error::CounterOverflow));
    assert_eq!(monitor.submitted(), t(2, u64::MAX));
}

#[test]
fn bounded_history_keeps_oldest_unconfirmed_cohort_age() {
    let mut monitor = Monitor::new(NonZeroUsize::new(2).unwrap());
    monitor.sample(s(0)).unwrap();
    monitor.record_submission(t(1, 4)).unwrap();
    let query = monitor.start_query(s(1)).unwrap();
    for second in 1..=8 {
        monitor.record_submission(t(1, 2)).unwrap();
        monitor.sample(s(second)).unwrap();
    }
    assert_eq!(monitor.history().map(|x| x.at).collect::<Vec<_>>(), vec![s(7), s(8)]);
    let snapshot = monitor.snapshot(s(10)).unwrap();
    assert_eq!(snapshot.oldest_outstanding_age, Some(s(10)));
    monitor.complete_query(query, s(10)).unwrap();
    let snapshot = monitor.sample(s(10)).unwrap();
    assert_eq!(snapshot.older_outstanding, t(8, 16));
    assert_eq!(snapshot.outstanding, t(8, 16));
    // Compaction can retain an earlier interval boundary, but must not rejuvenate old work.
    assert_eq!(snapshot.oldest_outstanding_age, Some(s(10)));
}

#[test]
fn older_outstanding_excludes_new_since_previous_sample() {
    let mut monitor = Monitor::default();
    monitor.record_submission(t(2, 4)).unwrap();
    monitor.sample(s(1)).unwrap();
    let query = monitor.start_query(s(1)).unwrap();
    monitor.record_submission(t(3, 6)).unwrap();
    monitor.complete_query(query, s(2)).unwrap();
    let snapshot = monitor.sample(s(3)).unwrap();
    assert_eq!(snapshot.older_outstanding, Totals::default());
    assert_eq!(snapshot.outstanding, t(3, 6));
    let query = monitor.start_query(s(3)).unwrap();
    monitor.complete_query(query, s(4)).unwrap();
    assert_eq!(monitor.sample(s(4)).unwrap().oldest_outstanding_age, None);
}

#[test]
fn ordered_query_can_capture_an_earlier_admitted_prefix() {
    let mut monitor = Monitor::default();
    monitor.record_submission(t(5, 15)).unwrap();
    let query = monitor.start_query_through(s(1), t(2, 6)).unwrap();
    assert_eq!(query.submitted, t(2, 6));
    monitor.complete_query(query, s(2)).unwrap();
    assert_eq!(monitor.confirmed(), t(2, 6));
    assert_eq!(monitor.snapshot(s(2)).unwrap().outstanding, t(3, 9));
    let before = monitor.snapshot(s(3)).unwrap();
    for invalid in [t(1, 6), t(2, 5), t(6, 15), t(5, 16)] {
        assert_eq!(monitor.start_query_through(s(3), invalid), Err(Error::InvalidPrefix));
        assert_eq!(monitor.snapshot(s(3)).unwrap(), before);
    }
    // Rejected prefixes did not consume identities or advance time.
    let next = monitor.start_query_through(s(2), t(2, 6)).unwrap();
    assert_eq!(next.id, query.id + 1);
    monitor.complete_query(next, s(2)).unwrap();
    let final_query = monitor.start_query(s(3)).unwrap();
    assert_eq!(final_query.submitted, t(5, 15));
}

#[test]
fn observed_clock_tracks_mutations_but_not_reads_or_failures() {
    let mut monitor = Monitor::default();
    assert_eq!(monitor.observed_at(), Duration::ZERO);
    monitor.sample(s(1)).unwrap();
    assert_eq!(monitor.observed_at(), s(1));
    monitor.snapshot(s(5)).unwrap();
    monitor.record_submission(t(1, 2)).unwrap();
    assert_eq!(monitor.observed_at(), s(1));
    let query = monitor.start_query(s(2)).unwrap();
    assert_eq!(monitor.observed_at(), s(2));
    assert_eq!(monitor.sample(s(1)), Err(Error::ClockReversed));
    assert_eq!(monitor.observed_at(), s(2));
    monitor.complete_query(query, s(3)).unwrap();
    assert_eq!(monitor.observed_at(), s(3));
}
