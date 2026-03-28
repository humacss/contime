use contime::{ContimeError, QueryResult, Snapshot, TestEvent, TestSnapshot, TestSnapshotContime};

#[test]
fn test_query_not_found() {
    let c = TestSnapshotContime::new(1, 1000);

    let handle = c.query_at(0, 999).unwrap();
    match handle.wait().unwrap() {
        QueryResult::NotFound => {}
        QueryResult::Found(_, _) => panic!("expected NotFound for unknown snapshot_id"),
    }
}

#[test]
fn test_query_not_found_via_at() {
    let c = TestSnapshotContime::new(1, 1000);

    match c.at::<TestSnapshot>(0, 999) {
        Err(ContimeError::NotFound) => {}
        Ok(_) => panic!("expected NotFound error"),
        Err(err) => panic!("unexpected error: {:?}", err),
    }
}

#[test]
fn test_query_at_various_times() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 2, 2, 10)).unwrap();
    c.apply_event(TestEvent::Positive(1, 5, 5, 20)).unwrap();
    c.apply_event(TestEvent::Positive(1, 8, 8, 30)).unwrap();

    // Before any events
    let (snap, _) = c.at::<TestSnapshot>(1, 1).unwrap();
    assert_eq!(snap.sum, 0);

    // After first event
    let (snap, _) = c.at::<TestSnapshot>(3, 1).unwrap();
    assert_eq!(snap.sum, 10);

    // After first two events
    let (snap, _) = c.at::<TestSnapshot>(6, 1).unwrap();
    assert_eq!(snap.sum, 30);

    // After all events
    let (snap, _) = c.at::<TestSnapshot>(9, 1).unwrap();
    assert_eq!(snap.sum, 60);
}

#[test]
fn test_query_before_any_events() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 10, 10, 50)).unwrap();

    let (snap, _) = c.at::<TestSnapshot>(0, 1).unwrap();
    assert_eq!(snap.sum, 0);
    assert_eq!(snap.items.len(), 0);
}

#[test]
fn test_query_at_exact_event_time() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 5, 5, 42)).unwrap();

    // Query at exactly t=5 — event at t=5 should NOT be included
    // (snapshot_at uses Bound::Excluded for the upper bound)
    let (snap, _) = c.at::<TestSnapshot>(5, 1).unwrap();
    assert_eq!(snap.sum, 0);

    // Query at t=6 — event at t=5 should be included
    let (snap, _) = c.at::<TestSnapshot>(6, 1).unwrap();
    assert_eq!(snap.sum, 42);
}

#[test]
fn test_query_handle_wait() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 7)).unwrap();

    let handle = c.query_at(2, 1).unwrap();
    match handle.wait().unwrap() {
        QueryResult::Found(snapshot_lane, _rx) => {
            let snap: TestSnapshot = snapshot_lane.into();
            assert_eq!(snap.sum, 7);
        }
        QueryResult::NotFound => panic!("expected Found"),
    }
}

#[test]
fn test_query_multiple_snapshot_ids() {
    let c = TestSnapshotContime::new(4, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 10)).unwrap();
    c.apply_event(TestEvent::Positive(2, 1, 2, 20)).unwrap();
    c.apply_event(TestEvent::Positive(3, 1, 3, 30)).unwrap();

    let (snap1, _) = c.at::<TestSnapshot>(2, 1).unwrap();
    let (snap2, _) = c.at::<TestSnapshot>(2, 2).unwrap();
    let (snap3, _) = c.at::<TestSnapshot>(2, 3).unwrap();

    assert_eq!(snap1.sum, 10);
    assert_eq!(snap1.id(), 1);
    assert_eq!(snap2.sum, 20);
    assert_eq!(snap2.id(), 2);
    assert_eq!(snap3.sum, 30);
    assert_eq!(snap3.id(), 3);
}
