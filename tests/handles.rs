use contime::{QueryResult, TestEvent, TestSnapshot, TestSnapshotContime};

#[test]
fn test_apply_handle_try_recv() {
    let c = TestSnapshotContime::new(1, 100000);

    let mut handle = c.send_event(TestEvent::Positive(1, 1, 1, 5)).unwrap();

    // Poll until complete
    loop {
        match handle.try_recv().unwrap() {
            Some(()) => break,
            None => std::thread::yield_now(),
        }
    }

    let (snap, _) = c.at::<TestSnapshot>(2, 1).unwrap();
    assert_eq!(snap.sum, 5);
}

#[test]
fn test_send_snapshot_handle() {
    let c = TestSnapshotContime::new(1, 100000);

    // First create the snapshot's history via an event
    c.apply_event(TestEvent::Positive(1, 1, 1, 10)).unwrap();

    // Send snapshot via handle
    let auth = TestSnapshot { id: 1, time: 0, sum: 99, items: vec![1, 2, 3] };
    let handle = c.send_snapshot(auth).unwrap();
    handle.wait().unwrap();

    // Query after the snapshot — auth at t=0 with sum=99, then event at t=1 adds 10
    let (snap, _) = c.at::<TestSnapshot>(2, 1).unwrap();
    assert_eq!(snap.sum, 109);
}

#[test]
fn test_send_event_then_query_ordering() {
    let c = TestSnapshotContime::new(1, 100000);

    // Send events via handles and wait for each
    for i in 0..5u128 {
        let handle = c.send_event(TestEvent::Positive(1, i as i64, i, 10)).unwrap();
        handle.wait().unwrap();
    }

    // All 5 events should be applied
    let (snap, _) = c.at::<TestSnapshot>(5, 1).unwrap();
    assert_eq!(snap.sum, 50);
}

#[test]
fn test_query_handle_try_recv() {
    let c = TestSnapshotContime::new(1, 100000);

    c.apply_event(TestEvent::Positive(1, 1, 1, 42)).unwrap();

    let handle = c.query_at(2, 1).unwrap();

    // Poll until result
    loop {
        match handle.try_recv().unwrap() {
            Some(QueryResult::Found(snapshot_lane, _rx)) => {
                let snap: TestSnapshot = snapshot_lane.into();
                assert_eq!(snap.sum, 42);
                break;
            }
            Some(QueryResult::NotFound) => panic!("expected Found"),
            None => std::thread::yield_now(),
        }
    }
}
