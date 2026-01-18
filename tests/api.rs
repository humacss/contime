use contime::{Contime, TestEventLanes, TestSnapshotLanes, TestEvent, TestSnapshot, Snapshot};

#[test]
fn test_api(
) {
    let c = Contime::<TestSnapshotLanes, TestEventLanes>::new();

    c.send(TestEvent::Positive(1, 0, 0, 0)).unwrap();

    match c.at::<TestSnapshot>(0, 1) {
        Ok((snapshot, _snapshot_rx)) => {
            assert_eq!(snapshot.id(), 1);
        },
        Err(err) => panic!("{:?}", err),
    }
}
