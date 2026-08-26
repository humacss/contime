use ahash::RandomState;
use crossbeam_channel::unbounded;

use super::{RoutePartitioner, Router};
use crate::{TestEvent, TestInputLanes, TestSnapshotLanes};

#[test]
fn dispatch_inputs_reports_only_affected_workers() {
    let mut router = Router::<TestSnapshotLanes, TestInputLanes>::new(8, 1_000_000);
    router.partitioner = RoutePartitioner::with_hasher(8, RandomState::with_seeds(1, 2, 3, 4));
    let first_snapshot_id = 1;
    let first_worker = router.worker_index(first_snapshot_id);
    let second_snapshot_id = (2..).find(|snapshot_id| router.worker_index(*snapshot_id) != first_worker).unwrap();
    let (response_tx, response_rx) = unbounded();

    let affected = router
        .dispatch_inputs(
            [TestEvent::Positive(first_snapshot_id, 10, 100, 1).into(), TestEvent::Positive(second_snapshot_id, 10, 200, 1).into()],
            Some(&response_tx),
        )
        .unwrap();

    assert_eq!(affected, 2);
    assert!(response_rx.recv().unwrap().is_empty());
    assert!(response_rx.recv().unwrap().is_empty());
    assert!(response_rx.try_recv().is_err());
}

#[test]
fn query_dispatch_returns_one_affected_worker() {
    let router = Router::<TestSnapshotLanes, TestInputLanes>::new(8, 1_000_000);
    let (response_tx, response_rx) = unbounded();

    let affected = router.dispatch_query(10, &[(0, 7)], &response_tx).unwrap();

    assert_eq!(affected, 1);
    assert_eq!(response_rx.recv().unwrap(), vec![(0, None)]);
    assert!(response_rx.try_recv().is_err());
}

#[test]
fn inspection_dispatch_returns_every_worker() {
    let router = Router::<TestSnapshotLanes, TestInputLanes>::new(8, 1_000_000);
    let (response_tx, response_rx) = unbounded();

    let affected = router.dispatch_inspection(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded, &response_tx).unwrap();

    assert_eq!(affected, 8);
    for _ in 0..8 {
        assert!(response_rx.recv().unwrap().is_empty());
    }
    assert!(response_rx.try_recv().is_err());
}

#[test]
fn advance_dispatch_returns_every_worker() {
    let router = Router::<TestSnapshotLanes, TestInputLanes>::new(8, 1_000_000);
    let (response_tx, response_rx) = unbounded();

    let affected = router.dispatch_advance(10, &response_tx).unwrap();

    assert_eq!(affected, 8);
    for _ in 0..8 {
        response_rx.recv().unwrap();
    }
    assert!(response_rx.try_recv().is_err());
}
