use contime::{Contime, Snapshot, TestEvent, TestEventLanes, TestSnapshot, TestSnapshotLanes};

#[test]
fn test_api() {
    let c = Contime::<TestSnapshotLanes, TestEventLanes>::new(1, 1_000);

    c.apply_event(TestEvent::Positive(1, 0, 0, 5)).unwrap();

    match c.at::<TestSnapshot>(1, 1) {
        Ok((snapshot, _snapshot_rx)) => {
            assert_eq!(snapshot.id(), 1);
            assert_eq!(snapshot.sum, 5);
        }
        Err(err) => panic!("{:?}", err),
    }
}
