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
        match c.apply_event(TestEvent::Positive(1, i as i64, i, 1)) {
            Ok(()) => {}
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
        match c.apply_event(TestEvent::Positive(1, i as i64, i, 1)) {
            Ok(()) => {
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
    let result = c.apply_event(TestEvent::Positive(1, last_applied + 200, (last_applied + 200) as u128, 1));
    assert!(result.is_ok(), "expected event to succeed after advance freed memory");
}

#[test]
fn test_advance_basic() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 10)).unwrap();
    c.apply_event(TestEvent::Positive(1, 5, 5, 20)).unwrap();
    c.apply_event(TestEvent::Positive(1, 10, 10, 30)).unwrap();

    // Advance should not panic or error
    c.advance_to(100).unwrap();
}

#[test]
fn test_advance_to_prunes_and_keeps_future_apply_query_working() {
    let c = TestSnapshotContime::new(2, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 10)).unwrap();
    c.apply_event(TestEvent::Positive(2, 1, 2, 20)).unwrap();

    c.advance_to(100).unwrap();

    // Should still be able to query and apply events after advance
    c.apply_event(TestEvent::Positive(1, 200, 200, 5)).unwrap();
    let snap = query_one(&c, 201, 1);
    assert_eq!(snap.sum, 15); // pruned history is carried by the promoted base snapshot
}
