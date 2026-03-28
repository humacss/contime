use contime::{Snapshot, TestEvent, TestSnapshot, TestSnapshotContime};

#[test]
fn test_apply_event_and_query() {
    let c = TestSnapshotContime::new(1, 1000);

    c.apply_event(TestEvent::Positive(1, 0, 0, 0)).unwrap();

    match c.at::<TestSnapshot>(0, 1) {
        Ok((snapshot, _reconciliation_rx)) => {
            assert_eq!(snapshot.id(), 1);
        }
        Err(err) => panic!("{:?}", err),
    }
}

#[test]
fn test_send_event() {
    let c = TestSnapshotContime::new(1, 1000);

    let handle = c.send_event(TestEvent::Positive(1, 0, 0, 5)).unwrap();
    handle.wait().unwrap();

    let (snapshot, _rx) = c.at::<TestSnapshot>(0, 1).unwrap();
    assert_eq!(snapshot.id(), 1);
    assert_eq!(snapshot.sum, 5);
}

#[test]
fn test_send_event_batch() {
    let c = TestSnapshotContime::new(1, 10000);

    let mut handles = Vec::new();
    for i in 0..10u128 {
        let handle = c.send_event(TestEvent::Positive(1, i as i64, i, 1)).unwrap();
        handles.push(handle);
    }

    for handle in handles {
        handle.wait().unwrap();
    }

    let (snapshot, _rx) = c.at::<TestSnapshot>(10, 1).unwrap();
    assert_eq!(snapshot.sum, 10);
}

#[test]
fn test_apply_snapshot_overrides_state() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 10)).unwrap();
    c.apply_event(TestEvent::Positive(1, 5, 5, 20)).unwrap();
    c.apply_event(TestEvent::Positive(1, 10, 10, 30)).unwrap();

    // Apply authoritative snapshot at t=3
    let auth = TestSnapshot { id: 1, time: 3, sum: 100, items: vec![99] };
    c.apply_snapshot(auth).unwrap();

    // Query after snapshot — should reflect snapshot + later events
    let (snap, _) = c.at::<TestSnapshot>(11, 1).unwrap();
    // auth=100 at t=3, then +20 at t=5, +30 at t=10 = 150
    assert_eq!(snap.sum, 150);
}
