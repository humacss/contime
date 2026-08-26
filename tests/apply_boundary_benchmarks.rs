use contime::{
    ContimeEvent, ContimeSnapshot, EventRejection, EventRejectionReason, RouterApplyBenchmark, SnapshotBatchBenchmark, TestEvent,
    TestInputLanes, TestSnapshot, TestSnapshotLanes, WorkerApplyBenchmark,
};

const MEMORY_BUDGET_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 48)]
pub struct MultiWorkerEvent {
    event_id: u128,
    time: i64,
    left_snapshot_id: u128,
    right_snapshot_id: u128,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [MultiWorkerEvent],
    id = [left_snapshot_id],
    bytes = 32,
    apply = {
        self.applied += batch.events.len() as u64;
    }
)]
struct LeftSnapshot {
    left_snapshot_id: u128,
    time: i64,
    applied: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [MultiWorkerEvent],
    id = [right_snapshot_id],
    bytes = 32,
    apply = {
        self.applied += batch.events.len() as u64;
    }
)]
struct RightSnapshot {
    right_snapshot_id: u128,
    time: i64,
    applied: u64,
}

contime::lanes! {
    mod multi_worker_lanes;
    snapshots [LeftSnapshot, RightSnapshot];
    routes [MultiWorkerEvent => [LeftSnapshot, RightSnapshot]];
}

fn inputs() -> Vec<TestInputLanes> {
    (1..=3).map(|event_id| TestEvent::Positive(7, 10, event_id, 1).into()).collect()
}

#[test]
fn worker_applies_pre_grouped_snapshot_batches_without_regrouping() {
    let worker = WorkerApplyBenchmark::<TestSnapshotLanes, TestInputLanes>::new(MEMORY_BUDGET_BYTES, 100);
    worker.warm_up(1);
    let batch = worker.prepare_snapshot_batch(7, inputs());

    assert!(worker.apply_snapshot_batches(vec![batch]).is_empty());
    let snapshot: TestSnapshot = worker.snapshot_at(7, 10).expect("worker should materialize the target snapshot").into();

    assert_eq!(snapshot.sum, 3);
}

#[test]
fn worker_rejects_a_complete_message_before_mutating_any_history() {
    let worker = WorkerApplyBenchmark::<TestSnapshotLanes, TestInputLanes>::new(1, 100);
    worker.warm_up(1);
    let batches = vec![
        worker.prepare_snapshot_batch(7, vec![TestEvent::Positive(7, 10, 11, 1).into()]),
        worker.prepare_snapshot_batch(9, vec![TestEvent::Positive(9, 10, 12, 1).into()]),
    ];

    assert_eq!(
        worker.apply_snapshot_batches(batches),
        vec![EventRejection::new(11, EventRejectionReason::MemoryFull), EventRejection::new(12, EventRejectionReason::MemoryFull),]
    );
    assert!(worker.snapshot_at(7, 10).is_none());
    assert!(worker.snapshot_at(9, 10).is_none());
}

#[test]
fn router_boundary_reports_the_worker_that_loses_a_shared_reservation() {
    let one_batch_budget = SnapshotBatchBenchmark::total_conservative_bytes::<TestSnapshotLanes, TestInputLanes, _>([TestEvent::Positive(
        1, 10, 11, 1,
    )
    .into()]);
    let router = RouterApplyBenchmark::<TestSnapshotLanes, TestInputLanes>::new(2, one_batch_budget, 100);
    let [first_snapshot_id, second_snapshot_id] = router.snapshot_ids_on_distinct_workers();
    let inputs = [TestEvent::Positive(first_snapshot_id, 10, 11, 1).into(), TestEvent::Positive(second_snapshot_id, 10, 12, 1).into()];

    let batches = router.prepare_snapshot_batches(inputs);
    let rejections = router.apply_snapshot_batches(batches);

    assert_eq!(rejections.len(), 1);
    assert_eq!(rejections[0].reason, EventRejectionReason::MemoryFull);
    let materialized = [router.snapshot_at(first_snapshot_id, 10).is_some(), router.snapshot_at(second_snapshot_id, 10).is_some()];
    assert_eq!(materialized.into_iter().filter(|exists| *exists).count(), 1);
}

#[test]
fn one_multi_route_event_reports_one_rejection_when_only_one_worker_applies_it() {
    let estimate =
        SnapshotBatchBenchmark::total_conservative_bytes::<multi_worker_lanes::SnapshotLanes, multi_worker_lanes::InputLanes, _>([
            MultiWorkerEvent { event_id: 77, time: 10, left_snapshot_id: 1, right_snapshot_id: 2 }.into(),
        ]);
    assert_eq!(estimate % 2, 0, "the two symmetric snapshot routes must have equal estimates");

    let router = RouterApplyBenchmark::<multi_worker_lanes::SnapshotLanes, multi_worker_lanes::InputLanes>::new(2, estimate / 2, 100);
    let [left_snapshot_id, right_snapshot_id] = router.snapshot_ids_on_distinct_workers();
    let event = MultiWorkerEvent { event_id: 77, time: 10, left_snapshot_id, right_snapshot_id };

    let rejections = router.apply_snapshot_batches(router.prepare_snapshot_batches([event.into()]));

    assert_eq!(rejections, vec![EventRejection::new(77, EventRejectionReason::MemoryFull)]);
    let materialized = [router.snapshot_at(left_snapshot_id, 10).is_some(), router.snapshot_at(right_snapshot_id, 10).is_some()];
    assert_eq!(materialized.into_iter().filter(|exists| *exists).count(), 1);
}

#[test]
fn direct_router_boundary_routes_one_batch_and_waits_for_its_worker() {
    let router = RouterApplyBenchmark::<TestSnapshotLanes, TestInputLanes>::new(1, MEMORY_BUDGET_BYTES, 100);
    let batches = router.prepare_snapshot_batches(inputs());

    assert!(router.apply_snapshot_batches(batches).is_empty());
    let snapshot: TestSnapshot = router.snapshot_at(7, 10).expect("router should return the target snapshot").into();

    assert_eq!(snapshot.sum, 3);
}
