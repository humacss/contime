use contime::{RouterApplyBenchmark, TestEvent, TestInputLanes, TestSnapshot, TestSnapshotLanes, WorkerApplyBenchmark};

const MEMORY_BUDGET_BYTES: u64 = 1024 * 1024;

fn inputs() -> Vec<TestInputLanes> {
    (1..=3).map(|event_id| TestEvent::Positive(7, 10, event_id, 1).into()).collect()
}

#[test]
fn direct_worker_boundary_applies_one_batch_and_replies_once() {
    let worker = WorkerApplyBenchmark::<TestSnapshotLanes, TestInputLanes>::new(MEMORY_BUDGET_BYTES, 100);
    let batch = worker.prepare_batch(7, inputs());

    assert!(worker.apply(batch).is_empty());
    let snapshot: TestSnapshot = worker.snapshot_at(7, 10).expect("worker should materialize the target snapshot").into();

    assert_eq!(snapshot.sum, 3);
}

#[test]
fn direct_router_boundary_routes_one_batch_and_waits_for_its_worker() {
    let router = RouterApplyBenchmark::<TestSnapshotLanes, TestInputLanes>::new(1, MEMORY_BUDGET_BYTES, 100);

    assert!(router.apply(inputs()).is_empty());
    let snapshot: TestSnapshot = router.snapshot_at(7, 10).expect("router should return the target snapshot").into();

    assert_eq!(snapshot.sum, 3);
}
