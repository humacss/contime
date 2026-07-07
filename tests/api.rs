use contime::{Contime, Snapshot, TestEvent, TestEventLanes, TestSnapshot, TestSnapshotLanes};

#[test]
fn test_api() {
    let c = Contime::<TestSnapshotLanes, TestEventLanes>::new(1, 1_000);

    c.apply_events([TestEvent::Positive(1, 0, 0, 5)]).unwrap();

    let snapshot: TestSnapshot = c.query_at(1, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.id(), 1);
    assert_eq!(snapshot.sum, 5);
}

#[test]
fn plural_apply_events_accepts_multiple_events() {
    let c = Contime::<TestSnapshotLanes, TestEventLanes>::new(1, 1_000);

    c.apply_events([TestEvent::Positive(1, 0, 0, 5), TestEvent::Positive(1, 1, 1, 7)]).unwrap();

    let snapshot: TestSnapshot = c.query_at(2, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 12);
}
