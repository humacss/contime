use contime::{ContimeError, TestEvent, TestSnapshot, TestSnapshotContime};

fn query_one(contime: &TestSnapshotContime, time: i64, snapshot_id: u128) -> TestSnapshot {
    contime.query_at(time, &[snapshot_id]).unwrap().pop().flatten().unwrap().into()
}

#[test]
fn test_memory_full() {
    let budget = 100u64;
    let c = TestSnapshotContime::new(1, budget);

    let mut hit_memory_full = false;
    for i in 0..1000u128 {
        match c.apply([TestEvent::Positive(1, i as i64, i, 1)].map(Into::into)) {
            Ok(_) => {}
            Err(ContimeError::MemoryFull) => {
                hit_memory_full = true;
                break;
            }
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
            Ok(_) => {
                last_applied = i as i64;
            }
            Err(ContimeError::MemoryFull) => break,
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

    let outcome = c.apply([TestEvent::Positive(1, 49, 1, 10)].map(Into::into)).unwrap();

    assert!(outcome.accepted_input_ids.is_empty());
    assert!(matches!(
        outcome.rejected_inputs.as_slice(),
        [contime::InputRejection {
            input_id: 1,
            input_time: 49,
            reason: contime::InputRejectionReason::BeforeHistoryHorizon { earliest_time: 50 },
        }]
    ));
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
