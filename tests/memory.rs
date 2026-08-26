use contime::{
    ContimeError, ContimeEvent, ContimeSnapshot, EventRejection, EventRejectionReason, TestEvent, TestSnapshot, TestSnapshotContime,
};

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 32)]
pub struct InlineEvent {
    event_id: u128,
    snapshot_id: u128,
    time: i64,
    value: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [InlineEvent],
    id = [snapshot_id],
    bytes = 32,
    apply = {
        for event in batch.events {
            let InlineSnapshotEvent::InlineEvent(event) = event;
            self.value += event.value;
        }
    }
)]
struct InlineSnapshot {
    snapshot_id: u128,
    time: i64,
    value: i64,
}

contime::lanes! {
    mod inline_lanes;
    snapshots [InlineSnapshot];
    routes [InlineEvent => [InlineSnapshot]];
}

fn query_one(contime: &TestSnapshotContime, time: i64, snapshot_id: u128) -> TestSnapshot {
    contime.query_at(time, &[snapshot_id]).unwrap().pop().flatten().unwrap().into()
}

#[test]
fn api_precheck_rejects_the_complete_apply_request() {
    let contime = TestSnapshotContime::new(1, 1);
    let rejections = contime.apply([TestEvent::Positive(1, 10, 10, 1), TestEvent::Positive(1, 10, 20, 1)].map(Into::into)).unwrap();

    assert_eq!(
        rejections,
        vec![EventRejection::new(10, EventRejectionReason::MemoryFull), EventRejection::new(20, EventRejectionReason::MemoryFull),]
    );
    assert!(contime.query_at(10, &[1]).unwrap()[0].is_none());
}

#[test]
fn api_precheck_returns_memory_full_error_for_send() {
    let contime = TestSnapshotContime::new(1, 1);

    assert!(matches!(contime.send([TestEvent::Positive(1, 10, 10, 1).into()]), Err(ContimeError::MemoryFull)));
}

#[test]
fn one_batch_reserves_cumulative_checkpoint_growth() {
    let contime = TestSnapshotContime::new(1, 1_000_000);
    let events = (1..=1_000_u128).map(|event_id| TestEvent::Positive(1, event_id as i64, event_id, 1).into());

    let rejections = contime.apply(events).expect("checkpoint growth must not disconnect the worker");

    assert!(rejections.is_empty());
    assert_eq!(query_one(&contime, 1_000, 1).items.len(), 1_000);
}

#[test]
fn zero_allocation_events_reserve_every_complete_checkpoint() {
    let contime = inline_lanes::Contime::new(1, 1_000_000);
    let events = (1..=1_000_u128).map(|event_id| InlineEvent { event_id, snapshot_id: 1, time: event_id as i64, value: 1 }.into());

    let rejections = contime.apply(events).expect("checkpoint bases must not disconnect the worker");

    assert!(rejections.is_empty());
    let snapshot: InlineSnapshot = contime.query_at(1_000, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.value, 1_000);
}

#[test]
fn late_event_reserves_growth_across_replayed_checkpoints() {
    let contime = TestSnapshotContime::new(1, 1_000_000);
    let events = (1..=1_000_u128).map(|event_id| TestEvent::Positive(1, event_id as i64, event_id, 1).into());
    assert!(contime.apply(events).unwrap().is_empty());

    let rejections =
        contime.apply([TestEvent::Positive(1, 50, 2_000, 1).into()]).expect("late-event replay growth must not disconnect the worker");

    assert!(rejections.is_empty());
    assert_eq!(query_one(&contime, 1_000, 1).items.len(), 1_001);
}

#[test]
fn test_memory_full() {
    let budget = 100u64;
    let c = TestSnapshotContime::new(1, budget);

    let mut hit_memory_full = false;
    for i in 0..1000u128 {
        match c.apply([TestEvent::Positive(1, i as i64, i, 1)].map(Into::into)) {
            Ok(rejections) if rejections.iter().any(|rejection| rejection.reason == contime::EventRejectionReason::MemoryFull) => {
                hit_memory_full = true;
                break;
            }
            Ok(_) => {}
            Err(err) => panic!("unexpected error: {:?}", err),
        }
    }

    assert!(hit_memory_full, "expected MemoryFull error with small budget");
}

#[test]
fn test_memory_full_then_advance_frees() {
    let budget = 500u64;
    let c = TestSnapshotContime::new(1, budget);

    // Fill up memory
    let mut last_applied = 0i64;
    for i in 0..1000u128 {
        match c.apply([TestEvent::Positive(1, i as i64, i, 1)].map(Into::into)) {
            Ok(rejections) if rejections.is_empty() => {
                last_applied = i as i64;
            }
            Ok(rejections) if rejections.iter().any(|rejection| rejection.reason == contime::EventRejectionReason::MemoryFull) => break,
            Ok(rejections) => panic!("unexpected rejections: {rejections:?}"),
            Err(err) => panic!("unexpected error: {:?}", err),
        }
    }

    assert!(last_applied > 0, "should have applied at least some events");

    // Advance to free old data
    c.advance_to(last_applied + 100).unwrap();

    // Should be able to apply new events now
    let result = c.apply([TestEvent::Positive(1, last_applied + 200, (last_applied + 200) as u128, 1)].map(Into::into));
    assert!(result.is_ok(), "expected event to succeed after advance freed memory");
}

#[test]
fn test_advance_basic() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply([TestEvent::Positive(1, 1, 1, 10), TestEvent::Positive(1, 5, 5, 20), TestEvent::Positive(1, 10, 10, 30)].map(Into::into))
        .unwrap();

    // Advance should not panic or error
    c.advance_to(100).unwrap();
}

#[test]
fn test_advance_to_prunes_and_keeps_future_apply_query_working() {
    let c = TestSnapshotContime::new(2, 100000);

    c.apply([TestEvent::Positive(1, 1, 1, 10), TestEvent::Positive(2, 1, 2, 20)].map(Into::into)).unwrap();

    c.advance_to(100).unwrap();

    // Should still be able to query and apply events after advance
    c.apply([TestEvent::Positive(1, 200, 200, 5)].map(Into::into)).unwrap();
    let snap = query_one(&c, 201, 1);
    assert_eq!(snap.sum, 15); // pruned history is carried by the retained replay anchor
}

#[test]
fn test_event_before_history_horizon_is_reported() {
    let c = TestSnapshotContime::with_history_horizon(1, 100000, 50);
    c.advance_to(100).unwrap();

    let rejections = c.apply([TestEvent::Positive(1, 49, 1, 10)].map(Into::into)).unwrap();

    assert_eq!(rejections, vec![contime::EventRejection::new(1, contime::EventRejectionReason::BeforeHistoryHorizon)]);
}

#[test]
fn test_event_at_history_horizon_is_accepted() {
    let c = TestSnapshotContime::with_history_horizon(1, 100000, 50);
    c.advance_to(100).unwrap();

    c.apply([TestEvent::Positive(1, 50, 1, 10)].map(Into::into)).unwrap();

    assert_eq!(query_one(&c, 100, 1).sum, 10);
}

#[test]
fn test_repeated_advance_to_uses_absolute_time() {
    let c = TestSnapshotContime::with_history_horizon(1, 100000, 50);
    c.apply([TestEvent::Positive(1, 30, 30, 30)].map(Into::into)).unwrap();

    c.advance_to(60).unwrap();
    c.advance_to(70).unwrap();
    c.apply([TestEvent::Positive(1, 25, 25, 5)].map(Into::into)).unwrap();

    assert_eq!(query_one(&c, 70, 1).items, vec![5, 30]);
}

#[test]
fn test_pruning_preserves_effects_without_a_checkpoint_before_horizon() {
    let c = TestSnapshotContime::with_history_horizon(1, 100000, 50);
    c.apply([TestEvent::Positive(1, 10, 10, 10), TestEvent::Positive(1, 100, 100, 100)].map(Into::into)).unwrap();

    c.advance_to(100).unwrap();
    c.apply([TestEvent::Positive(1, 60, 60, 60)].map(Into::into)).unwrap();

    let snapshot = query_one(&c, 100, 1);
    assert_eq!(snapshot.sum, 170);
    assert_eq!(snapshot.items, vec![10, 60, 100]);
}
