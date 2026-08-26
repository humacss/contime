use std::sync::{Arc, Barrier};

use contime::{
    ApplyInner, ApplyWrapper, EventRejection, EventRejectionReason, Input, InputBatch, InputRoute, Marker, TestEvent, TestSnapshot,
    TestSnapshotContime,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteMarker {
    event_id: u128,
    time: i64,
    snapshot_ids: Vec<u128>,
}

impl Input for RouteMarker {
    type Time = i64;

    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> Self::Time {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        32 + (self.snapshot_ids.len() * size_of::<u128>()) as u64
    }
}

impl Marker for RouteMarker {}

impl InputRoute for RouteMarker {
    fn visit_snapshot_ids<F>(&self, visit: &mut F)
    where
        F: FnMut(u128),
    {
        for &snapshot_id in &self.snapshot_ids {
            visit(snapshot_id);
        }
    }
}

contime::lanes! {
    mod marker_lanes;
    snapshots [TestSnapshot];
    markers [RouteMarker];
    routes [
        TestEvent => [TestSnapshot],
    ];
}

#[derive(Clone)]
struct WorkerTrace {
    worker_id: usize,
    tx: flume::Sender<(usize, u128)>,
}

impl ApplyWrapper<marker_lanes::SnapshotLanes> for WorkerTrace {
    fn apply_input_batch_wrapper(
        &mut self,
        batch: InputBatch<'_, marker_lanes::InputLanes>,
        apply_inner: &mut ApplyInner<'_, marker_lanes::SnapshotLanes>,
    ) {
        let snapshot_id = batch.snapshot_id;
        apply_inner.apply_input_batch(batch);
        self.tx.send((self.worker_id, snapshot_id)).unwrap();
    }
}

#[test]
fn concurrent_apply_calls_receive_only_their_own_rejections() {
    let contime = Arc::new(TestSnapshotContime::with_history_horizon(2, 1_000_000, 10));
    contime.advance_to(20).unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let first = {
        let contime = Arc::clone(&contime);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            contime.apply([TestEvent::Positive(1, 9, 101, 1).into()]).unwrap()
        })
    };
    let second = {
        let contime = Arc::clone(&contime);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            contime.apply([TestEvent::Positive(2, 9, 202, 1).into()]).unwrap()
        })
    };

    barrier.wait();
    assert_eq!(first.join().unwrap(), vec![EventRejection::new(101, EventRejectionReason::BeforeHistoryHorizon)]);
    assert_eq!(second.join().unwrap(), vec![EventRejection::new(202, EventRejectionReason::BeforeHistoryHorizon)]);
}

#[test]
fn identical_rejections_from_multiple_workers_are_returned_once() {
    let (trace_tx, trace_rx) = flume::unbounded();
    let contime = contime::Contime::<marker_lanes::SnapshotLanes, marker_lanes::InputLanes, WorkerTrace>::
        with_history_horizon_and_apply_context_factory(2, 1_000_000, 10, |worker_id| WorkerTrace {
            worker_id,
            tx: trace_tx.clone(),
        });
    let mut snapshot_by_worker = std::collections::BTreeMap::new();
    for snapshot_id in 1..=128 {
        contime.apply([TestEvent::Positive(snapshot_id, 1, 1_000 + snapshot_id, 1).into()]).unwrap();
        let (worker_id, observed_snapshot_id) = trace_rx.recv().unwrap();
        snapshot_by_worker.entry(worker_id).or_insert(observed_snapshot_id);
        if snapshot_by_worker.len() == 2 {
            break;
        }
    }
    assert_eq!(snapshot_by_worker.len(), 2);
    contime.advance_to(20).unwrap();

    let rejections =
        contime.apply([RouteMarker { event_id: 9_999, time: 9, snapshot_ids: snapshot_by_worker.into_values().collect() }.into()]).unwrap();

    assert_eq!(rejections, vec![EventRejection::new(9_999, EventRejectionReason::BeforeHistoryHorizon)]);
}
