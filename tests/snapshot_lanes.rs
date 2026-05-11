use flume::TryRecvError;

use contime::{Snapshot, TestEvent, TestSnapshot, TestSnapshotContime, TestSnapshotLanes};

fn snapshot_ids_by_worker(lanes_by_worker: &[Vec<TestSnapshotLanes>]) -> Vec<Vec<u128>> {
    lanes_by_worker.iter().map(|worker_lanes| worker_lanes.iter().map(Snapshot::id).collect::<Vec<_>>()).collect()
}

fn flatten_snapshots(lanes_by_worker: Vec<Vec<TestSnapshotLanes>>) -> Vec<TestSnapshot> {
    lanes_by_worker
        .into_iter()
        .flatten()
        .map(|lane| match lane {
            TestSnapshotLanes::TestSnapshot(snapshot) => snapshot,
        })
        .collect()
}

#[test]
fn snapshot_lanes_by_worker_empty_runtime_returns_one_group_per_worker() {
    let contime = TestSnapshotContime::new(3, 100_000);

    let lanes_by_worker = contime.snapshot_lanes_by_worker(0).unwrap();

    assert_eq!(lanes_by_worker, vec![vec![], vec![], vec![]]);
}

#[test]
fn snapshot_lanes_by_worker_returns_all_known_lanes_once() {
    let contime = TestSnapshotContime::new(4, 100_000);
    contime.apply_event(TestEvent::Positive(10, 1, 100, 10)).unwrap();
    contime.apply_event(TestEvent::Positive(20, 1, 200, 20)).unwrap();
    contime.apply_snapshot(TestSnapshot { id: 30, time: 1, items: vec![30], sum: 30 }).unwrap();

    let mut snapshots = flatten_snapshots(contime.snapshot_lanes_by_worker(2).unwrap());
    snapshots.sort_by_key(|snapshot| snapshot.id);

    assert_eq!(
        snapshots,
        vec![
            TestSnapshot { id: 10, time: 2, items: vec![10], sum: 10 },
            TestSnapshot { id: 20, time: 2, items: vec![20], sum: 20 },
            TestSnapshot { id: 30, time: 2, items: vec![30], sum: 30 },
        ]
    );
}

#[test]
fn snapshot_lanes_by_worker_sorts_lanes_by_snapshot_id_within_each_worker() {
    let contime = TestSnapshotContime::new(4, 100_000);
    for snapshot_id in [50, 10, 40, 20, 30] {
        contime.apply_event(TestEvent::Positive(snapshot_id, 1, snapshot_id, snapshot_id as u16)).unwrap();
    }

    let ids_by_worker = snapshot_ids_by_worker(&contime.snapshot_lanes_by_worker(2).unwrap());

    for worker_ids in ids_by_worker {
        let mut sorted = worker_ids.clone();
        sorted.sort_unstable();
        assert_eq!(worker_ids, sorted);
    }
}

#[test]
fn snapshot_lanes_by_worker_preserves_worker_grouping_shape() {
    let contime = TestSnapshotContime::new(5, 100_000);
    for snapshot_id in 0..32 {
        contime.apply_event(TestEvent::Positive(snapshot_id, 1, snapshot_id, 1)).unwrap();
    }

    let lanes_by_worker = contime.snapshot_lanes_by_worker(2).unwrap();
    let mut ids = snapshot_ids_by_worker(&lanes_by_worker).into_iter().flatten().collect::<Vec<_>>();
    ids.sort_unstable();

    assert_eq!(lanes_by_worker.len(), 5);
    assert_eq!(ids, (0..32).collect::<Vec<_>>());
}

#[test]
fn snapshot_lanes_by_worker_materializes_lanes_at_requested_time() {
    let contime = TestSnapshotContime::new(2, 100_000);
    contime.apply_event(TestEvent::Positive(1, 5, 50, 10)).unwrap();
    contime.apply_event(TestEvent::Positive(1, 8, 80, 20)).unwrap();

    let before_first = flatten_snapshots(contime.snapshot_lanes_by_worker(5).unwrap());
    let after_first = flatten_snapshots(contime.snapshot_lanes_by_worker(6).unwrap());
    let after_second = flatten_snapshots(contime.snapshot_lanes_by_worker(9).unwrap());

    assert_eq!(before_first, vec![TestSnapshot { id: 1, time: 5, items: vec![], sum: 0 }]);
    assert_eq!(after_first, vec![TestSnapshot { id: 1, time: 6, items: vec![10], sum: 10 }]);
    assert_eq!(after_second, vec![TestSnapshot { id: 1, time: 9, items: vec![10, 20], sum: 30 }]);
}

#[test]
fn snapshot_lanes_by_worker_does_not_emit_wakes() {
    let contime = TestSnapshotContime::new(2, 100_000);
    let wakes = contime.subscribe_wakes().unwrap();

    contime.apply_event(TestEvent::Positive(1, 5, 50, 10)).unwrap();
    assert!(wakes.try_recv().is_ok(), "setup event should emit a wake");

    let _ = contime.snapshot_lanes_by_worker(6).unwrap();

    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
}
