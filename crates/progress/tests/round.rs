use std::num::NonZeroUsize;

use contime_progress::MeasurementRound;

fn round(id: u64, cutoff: u64) -> MeasurementRound<u64> {
    MeasurementRound::new(id, cutoff, NonZeroUsize::new(2).unwrap())
}

#[test]
fn missing_participant_keeps_the_round_open() {
    let mut measurement = round(1, 100);
    assert_eq!(measurement.report(1, 0, None), None);
    assert!(!measurement.is_complete());
    assert_eq!(measurement.report(1, 1, None), Some(100));
    assert!(measurement.is_complete());
}

#[test]
fn pending_work_and_cutoff_both_bound_the_result() {
    let mut measurement = round(1, 100);
    assert_eq!(measurement.report(1, 0, Some(60)), None);
    assert_eq!(measurement.report(1, 1, Some(200)), Some(60));
    let mut measurement = round(2, 100);
    assert_eq!(measurement.report(2, 0, Some(150)), None);
    assert_eq!(measurement.report(2, 1, Some(200)), Some(100));
}

#[test]
fn duplicates_stale_rounds_and_unknown_participants_do_not_complete() {
    let mut measurement = round(2, 100);
    assert_eq!(measurement.report(1, 1, None), None);
    assert_eq!(measurement.report(2, 2, None), None);
    assert_eq!(measurement.report(2, 0, Some(80)), None);
    assert_eq!(measurement.report(2, 0, Some(10)), None);
    assert!(!measurement.is_complete());
    assert_eq!(measurement.report(2, 1, None), Some(80));
    assert_eq!(measurement.report(2, 1, Some(10)), None);
}

#[test]
fn admission_can_constrain_a_round_after_some_reports() {
    let mut measurement = round(1, 100);
    measurement.report(1, 0, None);
    measurement.constrain(20);
    measurement.constrain(80);
    assert_eq!(measurement.report(1, 1, None), Some(20));
}

#[test]
fn a_completed_round_cannot_be_changed_or_republished() {
    let mut measurement = round(1, 100);
    measurement.report(1, 0, None);
    assert_eq!(measurement.report(1, 1, None), Some(100));
    measurement.constrain(10);
    assert!(measurement.is_complete());
    assert_eq!(measurement.report(1, 0, None), None);
}

#[test]
fn complete_timestamp_order_is_preserved() {
    let mut measurement = MeasurementRound::new(1, (100_i64, 5_u32), NonZeroUsize::new(2).unwrap());
    assert_eq!(measurement.report(1, 0, Some((100, 4))), None);
    assert_eq!(measurement.report(1, 1, Some((100, 6))), Some((100, 4)));
}

#[test]
fn policies_reuse_measurement_with_independent_cutoffs_and_responses() {
    let mut pruning = round(1, 100);
    let mut lag_buffer = round(1, 300);
    for measurement in [&mut pruning, &mut lag_buffer] {
        assert_eq!(measurement.report(1, 0, Some(250)), None);
    }
    let mut pruned_to = 0;
    if let Some(boundary) = pruning.report(1, 1, None) {
        pruned_to = boundary;
    }
    let mut observations = Vec::new();
    if let Some(boundary) = lag_buffer.report(1, 1, None) {
        observations.push(boundary);
    }
    assert_eq!(pruned_to, 100);
    assert_eq!(observations, [250]);

    // A later observation may see newly admitted old work. Monotonic committed
    // progress belongs to the caller's coordination policy, not this reducer.
    let mut next = round(2, 300);
    next.report(2, 0, Some(200));
    assert_eq!(next.report(2, 1, None), Some(200));
}
