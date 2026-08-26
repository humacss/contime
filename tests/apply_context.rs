use std::sync::{Arc, Mutex};

use contime::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, Event, Input, InputBatch, Snapshot, SnapshotEvent};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ContextValueAt {
    entity_id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnContextValueChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    value: i32,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct ApplyTrace {
    applied: Vec<(u128, i64, i32)>,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct ApplyBatchTrace {
    applied: Vec<(u128, i64, usize)>,
}

#[derive(Clone)]
struct ApplyTraceSender {
    tx: flume::Sender<(u128, i64, i32, i32)>,
}

struct WorkerIdTraceSender {
    worker_id: usize,
    tx: flume::Sender<usize>,
}

struct GlobalTrace {
    label: &'static str,
    tx: flume::Sender<(&'static str, usize)>,
}

struct GlobalWorkerTraceSender {
    worker_id: usize,
    label: &'static str,
    tx: flume::Sender<(&'static str, usize)>,
}

#[derive(Clone)]
struct BlockingApplyTrace {
    entered_tx: flume::Sender<()>,
    release_rx: flume::Receiver<()>,
    applied: Arc<Mutex<Vec<u128>>>,
}

impl ContextValueAt {
    fn lane_id(entity_id: u128) -> u128 {
        entity_id
    }
}

impl Snapshot for ContextValueAt {
    type Time = i64;
    type Input = OnContextValueChanged;

    fn id(&self) -> u128 {
        Self::lane_id(self.entity_id)
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl Input for OnContextValueChanged {
    type Time = i64;

    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl Event for OnContextValueChanged {}

impl SnapshotEvent<ContextValueAt> for OnContextValueChanged {
    fn snapshot_id(&self) -> u128 {
        ContextValueAt::lane_id(self.entity_id)
    }

    fn set_snapshot_identity(&self, snapshot: &mut ContextValueAt) {
        snapshot.entity_id = self.entity_id;
    }
}

impl ApplyEvents<OnContextValueChanged> for ContextValueAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, OnContextValueChanged>) {
        if let Some(event) = batch.events.last().copied() {
            self.entity_id = event.entity_id;
            self.value = event.value;
        }
        self.time = batch.time;
    }
}

impl ApplyWrapper<ContextValueAt> for ApplyTrace {
    fn apply_input_batch_wrapper(
        &mut self,
        batch: InputBatch<'_, OnContextValueChanged>,
        apply_inner: &mut ApplyInner<'_, ContextValueAt>,
    ) {
        apply_inner.apply_input_batch(batch);
        let snapshot = apply_inner.snapshot();
        self.applied.push((snapshot.entity_id, snapshot.time, snapshot.value));
    }
}

impl ApplyWrapper<ContextValueAt> for ApplyTraceSender {
    fn apply_input_batch_wrapper(
        &mut self,
        batch: InputBatch<'_, OnContextValueChanged>,
        apply_inner: &mut ApplyInner<'_, ContextValueAt>,
    ) {
        let event = batch.inputs.last().copied().expect("after apply should receive non-empty bucket");
        apply_inner.apply_input_batch(batch);
        let snapshot = apply_inner.snapshot();
        self.tx.send((event.entity_id, event.time, event.value, snapshot.value)).unwrap();
    }
}

impl ApplyWrapper<ContextValueAt> for ApplyBatchTrace {
    fn apply_input_batch_wrapper(
        &mut self,
        batch: InputBatch<'_, OnContextValueChanged>,
        apply_inner: &mut ApplyInner<'_, ContextValueAt>,
    ) {
        self.applied.push((batch.snapshot_id, batch.time, batch.inputs.len()));
        apply_inner.apply_input_batch(batch);
    }
}

contime::lanes! {
    mod context_contime;
    snapshots [ContextValueAt];
    routes [
        OnContextValueChanged => [ContextValueAt],
    ];
}

use context_contime::{InputLanes, SnapshotLanes};

impl ApplyWrapper<SnapshotLanes> for ApplyTrace {
    fn apply_input_batch_wrapper(&mut self, batch: InputBatch<'_, InputLanes>, apply_inner: &mut ApplyInner<'_, SnapshotLanes>) {
        apply_inner.apply_input_batch(batch);
        let snapshot = apply_inner.snapshot();
        let SnapshotLanes::ContextValueAt(snapshot) = snapshot;
        self.applied.push((snapshot.entity_id, snapshot.time, snapshot.value));
    }
}

impl ApplyWrapper<SnapshotLanes> for ApplyTraceSender {
    fn apply_input_batch_wrapper(&mut self, batch: InputBatch<'_, InputLanes>, apply_inner: &mut ApplyInner<'_, SnapshotLanes>) {
        let event = batch.inputs.last().copied().expect("wrapper should receive non-empty bucket");
        let InputLanes::OnContextValueChanged(event) = event;
        let event = event.clone();
        apply_inner.apply_input_batch(batch);
        let snapshot = apply_inner.snapshot();
        let SnapshotLanes::ContextValueAt(snapshot) = snapshot;
        self.tx.send((event.entity_id, event.time, event.value, snapshot.value)).unwrap();
    }
}

impl ApplyWrapper<SnapshotLanes> for WorkerIdTraceSender {
    fn apply_input_batch_wrapper(&mut self, batch: InputBatch<'_, InputLanes>, apply_inner: &mut ApplyInner<'_, SnapshotLanes>) {
        apply_inner.apply_input_batch(batch);
        self.tx.send(self.worker_id).unwrap();
    }
}

impl ApplyWrapper<SnapshotLanes> for GlobalWorkerTraceSender {
    fn apply_input_batch_wrapper(&mut self, batch: InputBatch<'_, InputLanes>, apply_inner: &mut ApplyInner<'_, SnapshotLanes>) {
        apply_inner.apply_input_batch(batch);
        self.tx.send((self.label, self.worker_id)).unwrap();
    }
}

impl ApplyWrapper<SnapshotLanes> for BlockingApplyTrace {
    fn apply_input_batch_wrapper(&mut self, batch: InputBatch<'_, InputLanes>, apply_inner: &mut ApplyInner<'_, SnapshotLanes>) {
        self.entered_tx.send(()).unwrap();
        self.release_rx.recv().unwrap();
        let snapshot_id = batch.snapshot_id;
        apply_inner.apply_input_batch(batch);
        self.applied.lock().unwrap().push(snapshot_id);
    }
}

impl ApplyWrapper<SnapshotLanes> for ApplyBatchTrace {
    fn apply_input_batch_wrapper(&mut self, batch: InputBatch<'_, InputLanes>, apply_inner: &mut ApplyInner<'_, SnapshotLanes>) {
        self.applied.push((batch.snapshot_id, batch.time, batch.inputs.len()));
        apply_inner.apply_input_batch(batch);
    }
}

#[test]
fn context_free_apply_still_mutates_snapshot() {
    let event = OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 };
    let mut snapshot = ContextValueAt::default();

    <ContextValueAt as ApplyEvents<OnContextValueChanged>>::apply_events(
        &mut snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, history_input_count: 1, events: &[&event] },
    );

    assert_eq!(snapshot, ContextValueAt { entity_id: 3, time: 2, value: 4 });
}

#[test]
fn apply_wrapper_receives_snapshot_after_inner_apply_without_changing_snapshot_semantics() {
    let event = OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 };
    let mut snapshot = ContextValueAt::default();
    let mut context = ApplyTrace::default();

    {
        let mut apply_inner = ApplyInner::new(&mut snapshot, 1);
        <ApplyTrace as ApplyWrapper<ContextValueAt>>::apply_input_batch_wrapper(
            &mut context,
            InputBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, inputs: &[&event] },
            &mut apply_inner,
        );
    }

    assert_eq!(snapshot, ContextValueAt { entity_id: 3, time: 2, value: 4 });
    assert_eq!(context.applied, vec![(3, 2, 4)]);
}

#[test]
fn generated_lane_dispatch_works_through_apply_wrapper() {
    let event = InputLanes::OnContextValueChanged(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 });
    let mut snapshot = SnapshotLanes::ContextValueAt(ContextValueAt::default());
    let mut context = ApplyTrace::default();

    {
        let mut apply_inner = ApplyInner::new(&mut snapshot, 1);
        <ApplyTrace as ApplyWrapper<SnapshotLanes>>::apply_input_batch_wrapper(
            &mut context,
            InputBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, inputs: &[&event] },
            &mut apply_inner,
        );
    }

    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 2, value: 4 }));
    assert_eq!(context.applied, vec![(3, 2, 4)]);
}

#[test]
fn generated_lane_dispatch_passes_routed_snapshot_id_to_apply_batch() {
    let event = InputLanes::OnContextValueChanged(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 });
    let mut snapshot = SnapshotLanes::ContextValueAt(ContextValueAt::default());
    let mut context = ApplyBatchTrace::default();

    {
        let mut apply_inner = ApplyInner::new(&mut snapshot, 1);
        <ApplyBatchTrace as ApplyWrapper<SnapshotLanes>>::apply_input_batch_wrapper(
            &mut context,
            InputBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, inputs: &[&event] },
            &mut apply_inner,
        );
    }

    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 2, value: 4 }));
    assert_eq!(context.applied, vec![(3, 2, 1)]);
}

#[test]
fn contime_workers_use_configured_apply_context() {
    let (tx, rx) = flume::bounded(1);
    let contime =
        contime::Contime::<SnapshotLanes, InputLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });

    contime.apply([OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 }].map(Into::into)).unwrap();

    assert_eq!(rx.try_recv().unwrap(), (3, 2, 4, 4));
    let snapshot = contime.query_at(3, &[3]).unwrap().pop().flatten().unwrap();
    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 3, value: 4 }));
}

#[test]
fn contime_can_initialize_apply_context_per_worker() {
    let (tx, rx) = flume::bounded(64);
    let contime =
        contime::Contime::<SnapshotLanes, InputLanes, WorkerIdTraceSender>::new_with_apply_context_factory(2, 100_000, |worker_id| {
            WorkerIdTraceSender { worker_id, tx: tx.clone() }
        });

    for entity_id in 0..64 {
        contime
            .apply([OnContextValueChanged { event_id: entity_id, time: 1, entity_id, value: entity_id as i32 }].map(Into::into))
            .unwrap();
    }

    let mut worker_ids = std::collections::BTreeSet::new();
    while let Ok(worker_id) = rx.try_recv() {
        worker_ids.insert(worker_id);
    }

    assert_eq!(worker_ids, std::collections::BTreeSet::from([0, 1]));
}

#[test]
fn contime_can_initialize_worker_contexts_from_global_context() {
    let (tx, rx) = flume::bounded(64);
    let global_context = GlobalTrace { label: "global", tx };
    let contime = contime::Contime::<SnapshotLanes, InputLanes, GlobalWorkerTraceSender, GlobalTrace>::new_with_contexts(
        2,
        100_000,
        global_context,
        |worker_id, global| GlobalWorkerTraceSender { worker_id, label: global.label, tx: global.tx.clone() },
    );

    for entity_id in 0..64 {
        contime
            .apply([OnContextValueChanged { event_id: entity_id, time: 1, entity_id, value: entity_id as i32 }].map(Into::into))
            .unwrap();
    }

    let mut worker_ids = std::collections::BTreeSet::new();
    while let Ok((label, worker_id)) = rx.try_recv() {
        assert_eq!(label, "global");
        worker_ids.insert(worker_id);
    }

    assert_eq!(contime.global_context().label, "global");
    assert_eq!(worker_ids, std::collections::BTreeSet::from([0, 1]));
}

#[test]
fn out_of_order_apply_runs_after_apply_for_replayed_events() {
    let (tx, rx) = flume::bounded(8);
    let contime =
        contime::Contime::<SnapshotLanes, InputLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });

    contime.apply([OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 }].map(Into::into)).unwrap();
    contime.apply([OnContextValueChanged { event_id: 30, time: 30, entity_id: 3, value: 30 }].map(Into::into)).unwrap();
    assert_eq!(rx.try_recv().unwrap(), (3, 10, 10, 10));
    assert_eq!(rx.try_recv().unwrap(), (3, 30, 30, 30));

    contime.apply([OnContextValueChanged { event_id: 20, time: 20, entity_id: 3, value: 20 }].map(Into::into)).unwrap();

    assert_eq!(rx.try_recv().unwrap(), (3, 10, 10, 10));
    assert_eq!(rx.try_recv().unwrap(), (3, 20, 20, 20));
    assert_eq!(rx.try_recv().unwrap(), (3, 30, 30, 30));
    assert!(rx.try_recv().is_err());
}

#[test]
fn duplicate_apply_does_not_run_after_apply() {
    let (tx, rx) = flume::bounded(4);
    let contime =
        contime::Contime::<SnapshotLanes, InputLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });
    let event = OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 };

    contime.apply([event.clone()].map(Into::into)).unwrap();
    contime.apply([event].map(Into::into)).unwrap();

    assert_eq!(rx.try_recv().unwrap(), (3, 10, 10, 10));
    assert!(rx.try_recv().is_err());
}

#[test]
fn query_materialization_does_not_run_after_apply() {
    let (tx, rx) = flume::bounded(4);
    let contime =
        contime::Contime::<SnapshotLanes, InputLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });

    contime.apply([OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 }].map(Into::into)).unwrap();
    assert_eq!(rx.try_recv().unwrap(), (3, 10, 10, 10));

    let snapshot = contime.query_at(11, &[3]).unwrap().pop().flatten().unwrap();
    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 11, value: 10 }));
    assert!(rx.try_recv().is_err());
}

#[test]
fn send_event_returns_after_enqueue_without_waiting_for_apply() {
    let (entered_tx, entered_rx) = flume::bounded(1);
    let (release_tx, release_rx) = flume::bounded(1);
    let applied = Arc::new(Mutex::new(Vec::new()));
    let contime = contime::Contime::<SnapshotLanes, InputLanes, BlockingApplyTrace>::new_with_apply_context(
        1,
        100_000,
        BlockingApplyTrace { entered_tx, release_rx, applied: Arc::clone(&applied) },
    );

    contime.send([OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 }].map(Into::into)).unwrap();

    entered_rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
    assert!(applied.lock().unwrap().is_empty());

    release_tx.send(()).unwrap();
    let snapshot = contime.query_at(11, &[3]).unwrap().pop().flatten().unwrap();
    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 11, value: 10 }));
}
