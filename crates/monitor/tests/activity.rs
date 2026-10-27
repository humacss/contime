#![cfg(feature = "workers")]
use contime_monitor::{ApplicationActivity, WorkerCounters, WorkerReading, WorkerStatus};
use std::time::Duration;
fn reading(at: u64, counts: &[(u64, Option<u64>)]) -> WorkerReading<u64> {
    WorkerReading {
        started_at: Duration::from_secs(at),
        completed_at: Duration::from_secs(at),
        workers: counts
            .iter()
            .enumerate()
            .map(|(worker_id, (inserted, replay))| WorkerStatus {
                worker_id,
                history_stores: 0,
                metadata_only: 0,
                pending_histories: 0,
                due_histories: 0,
                processing_target: 0,
                admission_horizon: 0,
                pending_prune_passes: 0,
                awaiting_resolution: None,
                counters: Some(WorkerCounters { inserted_applications: *inserted, replay_applications: *replay, saturated: false }),
            })
            .collect(),
    }
}
#[test]
fn rates_use_real_counter_deltas_and_wall_time_not_logical_time() {
    let first = reading(1, &[(100, Some(120)), (100, Some(120))]);
    let second = reading(3, &[(300, Some(420)), (500, Some(720))]);
    let baseline = ApplicationActivity::between(None, &first);
    assert_eq!(baseline.inserted_applications, Some(200));
    assert_eq!(baseline.inserted_per_second, None);
    let activity = ApplicationActivity::between(Some(&first), &second);
    assert_eq!(activity.inserted_applications, Some(800));
    assert_eq!(activity.replay_applications, Some(1140));
    assert_eq!(activity.inserted_per_second, Some(300.0));
    assert_eq!(activity.replay_per_second, Some(450.0));
    assert_eq!(activity.workers[0].inserted_per_second, Some(100.0));
    assert_eq!(activity.workers[1].replay_per_second, Some(300.0));
}
#[test]
fn disabled_unknown_saturated_and_reset_counts_are_not_zero_rates() {
    let previous = reading(1, &[(100, Some(200))]);
    let mut now = reading(2, &[(200, None)]);
    let unknown = ApplicationActivity::between(Some(&previous), &now);
    assert_eq!(unknown.inserted_per_second, Some(100.0));
    assert_eq!(unknown.replay_applications, None);
    assert_eq!(unknown.replay_per_second, None);
    now.workers[0].counters = None;
    let off = ApplicationActivity::between(Some(&previous), &now);
    assert_eq!(off.inserted_applications, None);
    assert_eq!(off.inserted_per_second, None);
    let reset = ApplicationActivity::between(Some(&previous), &reading(2, &[(10, Some(20))]));
    assert_eq!(reset.inserted_per_second, None);
    assert_eq!(reset.replay_per_second, None);
    now = reading(2, &[(u64::MAX, Some(u64::MAX))]);
    now.workers[0].counters.as_mut().unwrap().saturated = true;
    let saturated = ApplicationActivity::between(Some(&previous), &now);
    assert!(saturated.saturated);
    assert_eq!(saturated.inserted_per_second, None);
    assert_eq!(saturated.replay_per_second, None);
}
#[test]
fn zero_backwards_time_and_worker_membership_changes_have_no_aggregate_rate() {
    let previous = reading(2, &[(100, Some(100))]);
    for at in [1, 2] {
        assert_eq!(ApplicationActivity::between(Some(&previous), &reading(at, &[(200, Some(200))])).inserted_per_second, None);
    }
    assert_eq!(ApplicationActivity::between(Some(&previous), &reading(3, &[(200, Some(200)), (100, Some(100))])).inserted_per_second, None);
    let mut different = reading(3, &[(200, Some(200))]);
    different.workers[0].worker_id = 9;
    assert_eq!(ApplicationActivity::between(Some(&previous), &different).inserted_per_second, None);
    let empty = reading(4, &[]);
    assert_eq!(ApplicationActivity::between(Some(&previous), &empty).inserted_applications, None);
}

#[test]
fn one_decreasing_counter_invalidates_both_rates_and_a_later_reading_recovers() {
    let before = reading(1, &[(100, Some(200))]);
    let restart = reading(2, &[(10, Some(300))]);
    let activity = ApplicationActivity::between(Some(&before), &restart);
    assert_eq!(activity.inserted_per_second, None);
    assert_eq!(activity.replay_per_second, None);
    let recovered = ApplicationActivity::between(Some(&restart), &reading(3, &[(20, Some(320))]));
    assert_eq!(recovered.inserted_per_second, Some(10.0));
    assert_eq!(recovered.replay_per_second, Some(20.0));
}

#[test]
fn idle_intervals_have_real_zero_rates() {
    let before = reading(1, &[(10, Some(20))]);
    let after = reading(2, &[(10, Some(20))]);
    let idle = ApplicationActivity::between(Some(&before), &after);
    assert_eq!(idle.inserted_per_second, Some(0.0));
    assert_eq!(idle.replay_per_second, Some(0.0));
}
