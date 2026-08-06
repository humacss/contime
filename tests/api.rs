use contime::{Contime, Snapshot, TestEvent, TestInputLanes, TestSnapshot, TestSnapshotLanes};

#[test]
#[should_panic(expected = "worker_count must be greater than zero")]
fn zero_workers_are_rejected_during_construction() {
    let _contime = Contime::<TestSnapshotLanes, TestInputLanes>::new(0, 1_000);
}

#[test]
fn test_api() {
    let c = Contime::<TestSnapshotLanes, TestInputLanes>::new(1, 1_000);

    c.apply([TestEvent::Positive(1, 0, 0, 5)].map(Into::into)).unwrap();

    let snapshot: TestSnapshot = c.query_at(1, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.id(), 1);
    assert_eq!(snapshot.sum, 5);
}

#[test]
fn plural_apply_events_accepts_multiple_events() {
    let c = Contime::<TestSnapshotLanes, TestInputLanes>::new(1, 1_000);

    c.apply([TestEvent::Positive(1, 0, 0, 5), TestEvent::Positive(1, 1, 1, 7)].map(Into::into)).unwrap();

    let snapshot: TestSnapshot = c.query_at(2, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 12);
}
