use super::*;
use contime::contime_monitor::WorkerReading;
use contime::contime_worker::{WorkerCounters, WorkerStatus};

fn reading(at: u64, applications: u64, pending: usize, target: u64) -> WorkerReading<Time> {
    WorkerReading {
        started_at: Duration::from_millis(at),
        completed_at: Duration::from_millis(at),
        workers: vec![WorkerStatus {
            worker_id: 0,
            history_stores: 1,
            metadata_only: 0,
            pending_histories: pending,
            due_histories: pending,
            processing_target: Time(target),
            admission_horizon: Time(0),
            pending_prune_passes: 0,
            awaiting_resolution: None,
            counters: Some(WorkerCounters {
                inserted_applications: applications,
                replay_applications: Some(u64::MAX / 2),
                saturated: false,
            }),
        }],
    }
}
fn fill(controller: &mut Controller, at: u64, target: u64) -> u64 {
    let mut total = 0;
    loop {
        let count = controller.allowance(Duration::from_millis(at));
        if count == 0 {
            break;
        }
        controller.submitted(count, count * controller.fanout, Time(target), Duration::from_millis(at)).unwrap();
        total += count;
    }
    total
}
#[test]
fn replay_totals_and_insertions_without_completed_replay_never_release_capacity() {
    let mut controller = Controller::new(100_000, 3, true);
    let events = fill(&mut controller, 0, 10);
    assert!(events > 0);
    assert!(!controller.observe(&reading(10, events * 3, 1, 10)).unwrap());
    assert!(!controller.observe(&reading(11, events * 3 - 1, 0, 10)).unwrap());
    assert!(!controller.observe(&reading(12, events * 3, 0, 9)).unwrap());
    assert_eq!(controller.completed(), 0);
    assert_eq!(controller.outstanding(), events);
    assert_eq!(controller.allowance(Duration::from_millis(12)), 0);
    assert!(controller.observe(&reading(20, events * 3, 0, 10)).unwrap());
    assert_eq!(controller.completed(), events);
    assert_eq!(controller.outstanding(), 0);
}
#[test]
fn fast_completion_increases_load_and_a_slow_later_window_decreases_it() {
    let mut controller = Controller::new(100_000, 1, true);
    let first = fill(&mut controller, 0, 1);
    assert!(controller.observe(&reading(20, first, 0, 1)).unwrap());
    let increased = controller.rate();
    assert!(increased > 100_000);
    let second = fill(&mut controller, 25, 2);
    assert!(!controller.observe(&reading(30, first + second, 1, 2)).unwrap());
    assert!(controller.observe(&reading(225, first + second, 0, 2)).unwrap());
    assert!(controller.rate() < increased);
    assert_eq!(controller.completed(), first + second);
    assert!(!controller.observe(&reading(230, first + second, 0, 2)).unwrap());
}
#[test]
fn stale_readings_missing_or_saturated_counters_do_not_confirm_work() {
    let mut controller = Controller::new(100_000, 1, true);
    let count = fill(&mut controller, 10, 1);
    assert!(!controller.observe(&reading(9, count, 0, 1)).unwrap());
    let mut missing = reading(20, count, 0, 1);
    missing.workers[0].counters = None;
    assert!(controller.observe(&missing).is_err());
    let mut saturated = reading(20, count, 0, 1);
    saturated.workers[0].counters.as_mut().unwrap().saturated = true;
    assert!(controller.observe(&saturated).is_err());
    assert_eq!(controller.completed(), 0);
}
#[test]
fn stopping_confirms_a_partial_window_without_submitting_the_rest() {
    let mut controller = Controller::new(100_000, 1, true);
    controller.allowance(Duration::ZERO);
    controller.submitted(17, 17 * controller.fanout, Time(1), Duration::ZERO).unwrap();
    controller.stop(Duration::from_millis(1));
    assert_eq!(controller.allowance(Duration::from_millis(2)), 0);
    assert!(controller.observe(&reading(3, 17, 0, 1)).unwrap());
    assert_eq!(controller.completed(), 17);
    assert_eq!(controller.outstanding(), 0);
}
#[test]
fn numeric_rate_is_paced_and_not_adjusted_by_monitor_completion() {
    let mut controller = Controller::new(1_000, 1, false);
    assert_eq!(controller.allowance(Duration::ZERO), 0);
    assert_eq!(controller.allowance(Duration::from_millis(10)), 10);
    controller.submitted(10, 10 * controller.fanout, Time(1), Duration::from_millis(10)).unwrap();
    assert_eq!(controller.allowance(Duration::from_millis(10)), 0);
    controller.stop(Duration::from_millis(11));
    assert!(controller.observe(&reading(20, 10, 0, 1)).unwrap());
    assert_eq!(controller.rate(), 1_000);
}

#[test]
fn controller_probes_up_again_after_a_temporary_slowdown() {
    let mut controller = Controller::new(100_000, 1, true);
    let first = fill(&mut controller, 0, 1);
    controller.observe(&reading(20, first, 0, 1)).unwrap();
    let second = fill(&mut controller, 25, 2);
    controller.observe(&reading(225, first + second, 0, 2)).unwrap();
    let reduced = controller.rate();
    let third = fill(&mut controller, 230, 3);
    assert!(controller.observe(&reading(250, first + second + third, 0, 3)).unwrap());
    assert!(controller.rate() > reduced);
}

#[test]
fn every_worker_must_have_finished_its_own_inserted_work() {
    let mut controller = Controller::new(100_000, 1, true);
    let count = fill(&mut controller, 0, 10);
    let mut observed = reading(20, count / 2, 0, 10);
    let mut other = observed.workers[0].clone();
    other.worker_id = 1;
    other.counters.as_mut().unwrap().inserted_applications = count - count / 2;
    other.pending_histories = 1;
    observed.workers.push(other);
    assert!(!controller.observe(&observed).unwrap());
    assert_eq!(controller.completed(), 0);
    observed.workers[1].pending_histories = 0;
    assert!(controller.observe(&observed).unwrap());
    assert_eq!(controller.completed(), count);
}

#[test]
fn actual_applications_drive_confirmation_with_variable_fanout() {
    let mut controller = Controller::new(100_000, 4, true);
    controller.allowance(Duration::ZERO);
    controller.submitted(2, 5, Time(1), Duration::ZERO).unwrap();
    controller.stop(Duration::from_millis(1));
    assert!(!controller.observe(&reading(2, 4, 0, 1)).unwrap());
    assert!(controller.observe(&reading(3, 5, 0, 1)).unwrap());
    assert_eq!(controller.completed(), 2);
}

#[test]
fn reversed_delivery_waits_for_two_events_unless_the_window_has_one_slot() {
    let mut controller = Controller::new(100, 1, false);
    controller.reverse_groups();
    assert_eq!(controller.allowance(Duration::from_millis(10)), 1);
    assert_eq!(controller.minimum_submission(), 2);
    assert_eq!(controller.allowance(Duration::from_millis(20)), 2);
    controller.submitted(2, 2, Time(1), Duration::from_millis(20)).unwrap();
    let mut single = Controller::new(100, 1, false);
    single.reverse_groups();
    single.limit_sources(1);
    assert_eq!(single.allowance(Duration::from_millis(10)), 1);
    assert_eq!(single.minimum_submission(), 1);
}
