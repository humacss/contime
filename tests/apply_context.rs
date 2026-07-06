use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use contime::{ApplyBatch, ApplyDecision, ApplyEvents, ApplyInner, ApplyWrapper, Event, Snapshot, SnapshotEvent};

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
    type Event = OnContextValueChanged;

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

    fn from_event(event: &Self::Event) -> Self {
        Self { entity_id: event.entity_id, time: event.time, value: event.value }
    }
}

impl Event for OnContextValueChanged {
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

impl SnapshotEvent<ContextValueAt> for OnContextValueChanged {
    fn snapshot_id(&self) -> u128 {
        ContextValueAt::lane_id(self.entity_id)
    }
}

impl ApplyEvents for ContextValueAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>) {
        if let Some(event) = batch.events.last().copied() {
            self.entity_id = event.entity_id;
            self.value = event.value;
        }
        self.time = batch.time;
    }
}

impl ApplyWrapper<ContextValueAt> for ApplyTrace {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut ContextValueAt,
        batch: ApplyBatch<'_, OnContextValueChanged>,
        apply_inner: ApplyInner<ContextValueAt>,
    ) -> Result<ApplyDecision, Self::Error> {
        apply_inner.apply_event_batch(snapshot, batch);
        self.applied.push((snapshot.entity_id, snapshot.time, snapshot.value));
        Ok(ApplyDecision::Continue)
    }
}

impl ApplyWrapper<ContextValueAt> for ApplyTraceSender {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut ContextValueAt,
        batch: ApplyBatch<'_, OnContextValueChanged>,
        apply_inner: ApplyInner<ContextValueAt>,
    ) -> Result<ApplyDecision, Self::Error> {
        let event = batch.events.last().copied().expect("after apply should receive non-empty bucket");
        apply_inner.apply_event_batch(snapshot, batch);
        self.tx.send((event.entity_id, event.time, event.value, snapshot.value)).unwrap();
        Ok(ApplyDecision::Continue)
    }
}

impl ApplyWrapper<ContextValueAt> for ApplyBatchTrace {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut ContextValueAt,
        batch: ApplyBatch<'_, OnContextValueChanged>,
        apply_inner: ApplyInner<ContextValueAt>,
    ) -> Result<ApplyDecision, Self::Error> {
        self.applied.push((batch.snapshot_id, batch.time, batch.events.len()));
        apply_inner.apply_event_batch(snapshot, batch);
        Ok(ApplyDecision::Continue)
    }
}

contime::lanes! {
    mod context_contime;
    snapshots [ContextValueAt];
    routes [
        OnContextValueChanged => [ContextValueAt],
    ];
}

use context_contime::{EventLanes, SnapshotLanes};

impl ApplyWrapper<SnapshotLanes> for ApplyTrace {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut SnapshotLanes,
        batch: ApplyBatch<'_, EventLanes>,
        apply_inner: ApplyInner<SnapshotLanes>,
    ) -> Result<ApplyDecision, Self::Error> {
        apply_inner.apply_event_batch(snapshot, batch);
        let SnapshotLanes::ContextValueAt(snapshot) = snapshot;
        self.applied.push((snapshot.entity_id, snapshot.time, snapshot.value));
        Ok(ApplyDecision::Continue)
    }
}

impl ApplyWrapper<SnapshotLanes> for ApplyTraceSender {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut SnapshotLanes,
        batch: ApplyBatch<'_, EventLanes>,
        apply_inner: ApplyInner<SnapshotLanes>,
    ) -> Result<ApplyDecision, Self::Error> {
        let event = batch.events.last().copied().expect("wrapper should receive non-empty bucket");
        let EventLanes::OnContextValueChanged(event) = event;
        let event = event.clone();
        apply_inner.apply_event_batch(snapshot, batch);
        let SnapshotLanes::ContextValueAt(snapshot) = snapshot;
        self.tx.send((event.entity_id, event.time, event.value, snapshot.value)).unwrap();
        Ok(ApplyDecision::Continue)
    }
}

impl ApplyWrapper<SnapshotLanes> for WorkerIdTraceSender {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut SnapshotLanes,
        batch: ApplyBatch<'_, EventLanes>,
        apply_inner: ApplyInner<SnapshotLanes>,
    ) -> Result<ApplyDecision, Self::Error> {
        apply_inner.apply_event_batch(snapshot, batch);
        self.tx.send(self.worker_id).unwrap();
        Ok(ApplyDecision::Continue)
    }
}

impl ApplyWrapper<SnapshotLanes> for GlobalWorkerTraceSender {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut SnapshotLanes,
        batch: ApplyBatch<'_, EventLanes>,
        apply_inner: ApplyInner<SnapshotLanes>,
    ) -> Result<ApplyDecision, Self::Error> {
        apply_inner.apply_event_batch(snapshot, batch);
        self.tx.send((self.label, self.worker_id)).unwrap();
        Ok(ApplyDecision::Continue)
    }
}

impl ApplyWrapper<SnapshotLanes> for BlockingApplyTrace {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut SnapshotLanes,
        batch: ApplyBatch<'_, EventLanes>,
        apply_inner: ApplyInner<SnapshotLanes>,
    ) -> Result<ApplyDecision, Self::Error> {
        self.entered_tx.send(()).unwrap();
        self.release_rx.recv().unwrap();
        apply_inner.apply_event_batch(snapshot, batch);
        self.applied.lock().unwrap().push(batch.snapshot_id);
        Ok(ApplyDecision::Continue)
    }
}

impl ApplyWrapper<SnapshotLanes> for ApplyBatchTrace {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut SnapshotLanes,
        batch: ApplyBatch<'_, EventLanes>,
        apply_inner: ApplyInner<SnapshotLanes>,
    ) -> Result<ApplyDecision, Self::Error> {
        self.applied.push((batch.snapshot_id, batch.time, batch.events.len()));
        apply_inner.apply_event_batch(snapshot, batch);
        Ok(ApplyDecision::Continue)
    }
}

struct EarlyExitAtTime {
    exit_time: i64,
    batches: Vec<i64>,
}

impl ApplyWrapper<SnapshotLanes> for EarlyExitAtTime {
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut SnapshotLanes,
        batch: ApplyBatch<'_, EventLanes>,
        apply_inner: ApplyInner<SnapshotLanes>,
    ) -> Result<ApplyDecision, Self::Error> {
        self.batches.push(batch.time);
        if batch.time == self.exit_time {
            return Ok(ApplyDecision::EarlyExit);
        }
        apply_inner.apply_event_batch(snapshot, batch);
        Ok(ApplyDecision::Continue)
    }
}

#[test]
fn context_free_apply_still_mutates_snapshot() {
    let event = OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 };
    let mut snapshot = ContextValueAt::default();

    <ContextValueAt as ApplyEvents>::apply_events(
        &mut snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[&event] },
    );

    assert_eq!(snapshot, ContextValueAt { entity_id: 3, time: 2, value: 4 });
}

#[test]
fn apply_wrapper_can_exit_replay_early() {
    let snapshot = SnapshotLanes::ContextValueAt(ContextValueAt::default());
    let (mut history, _) = contime::SnapshotHistory::new(snapshot, 0, 1000);
    let mut context = EarlyExitAtTime { exit_time: 20, batches: Vec::new() };

    history
        .apply_event_batch(
            vec![
                EventLanes::OnContextValueChanged(OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 }),
                EventLanes::OnContextValueChanged(OnContextValueChanged { event_id: 20, time: 20, entity_id: 3, value: 20 }),
                EventLanes::OnContextValueChanged(OnContextValueChanged { event_id: 30, time: 30, entity_id: 3, value: 30 }),
            ],
            &mut context,
        )
        .unwrap();

    assert_eq!(context.batches, vec![10, 20]);
    assert_eq!(history.snapshot_only_at(30), SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 30, value: 10 }));
}

#[test]
fn apply_wrapper_receives_snapshot_after_inner_apply_without_changing_snapshot_semantics() {
    let event = OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 };
    let mut snapshot = ContextValueAt::default();
    let mut context = ApplyTrace::default();

    <ApplyTrace as ApplyWrapper<ContextValueAt>>::apply_event_batch_wrapper(
        &mut context,
        &mut snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[&event] },
        ApplyInner::default(),
    )
    .unwrap();

    assert_eq!(snapshot, ContextValueAt { entity_id: 3, time: 2, value: 4 });
    assert_eq!(context.applied, vec![(3, 2, 4)]);
}

#[test]
fn generated_lane_dispatch_works_through_apply_wrapper() {
    let event = EventLanes::OnContextValueChanged(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 });
    let mut snapshot = SnapshotLanes::ContextValueAt(ContextValueAt::default());
    let mut context = ApplyTrace::default();

    <ApplyTrace as ApplyWrapper<SnapshotLanes>>::apply_event_batch_wrapper(
        &mut context,
        &mut snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[&event] },
        ApplyInner::default(),
    )
    .unwrap();

    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 2, value: 4 }));
    assert_eq!(context.applied, vec![(3, 2, 4)]);
}

#[test]
fn generated_lane_dispatch_passes_routed_snapshot_id_to_apply_batch() {
    let event = EventLanes::OnContextValueChanged(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 });
    let mut snapshot = SnapshotLanes::ContextValueAt(ContextValueAt::default());
    let mut context = ApplyBatchTrace::default();

    <ApplyBatchTrace as ApplyWrapper<SnapshotLanes>>::apply_event_batch_wrapper(
        &mut context,
        &mut snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[&event] },
        ApplyInner::default(),
    )
    .unwrap();

    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 2, value: 4 }));
    assert_eq!(context.applied, vec![(3, 2, 1)]);
}

#[test]
fn contime_workers_use_configured_apply_context() {
    let (tx, rx) = flume::bounded(1);
    let contime =
        contime::Contime::<SnapshotLanes, EventLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });

    contime.apply_event(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 }).unwrap();

    assert_eq!(rx.try_recv().unwrap(), (3, 2, 4, 4));
    let snapshot = contime.query_at(3, &[3]).unwrap().pop().flatten().unwrap();
    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 3, value: 4 }));
}

#[test]
fn contime_can_initialize_apply_context_per_worker() {
    let (tx, rx) = flume::bounded(64);
    let contime =
        contime::Contime::<SnapshotLanes, EventLanes, WorkerIdTraceSender>::new_with_apply_context_factory(2, 100_000, |worker_id| {
            WorkerIdTraceSender { worker_id, tx: tx.clone() }
        });

    for entity_id in 0..64 {
        contime.apply_event(OnContextValueChanged { event_id: entity_id, time: 1, entity_id, value: entity_id as i32 }).unwrap();
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
    let contime = contime::Contime::<SnapshotLanes, EventLanes, GlobalWorkerTraceSender, GlobalTrace>::new_with_contexts(
        2,
        100_000,
        global_context,
        |worker_id, global| GlobalWorkerTraceSender { worker_id, label: global.label, tx: global.tx.clone() },
    );

    for entity_id in 0..64 {
        contime.apply_event(OnContextValueChanged { event_id: entity_id, time: 1, entity_id, value: entity_id as i32 }).unwrap();
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
        contime::Contime::<SnapshotLanes, EventLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });

    contime.apply_event(OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 }).unwrap();
    contime.apply_event(OnContextValueChanged { event_id: 30, time: 30, entity_id: 3, value: 30 }).unwrap();
    assert_eq!(rx.try_recv().unwrap(), (3, 10, 10, 10));
    assert_eq!(rx.try_recv().unwrap(), (3, 30, 30, 30));

    contime.apply_event(OnContextValueChanged { event_id: 20, time: 20, entity_id: 3, value: 20 }).unwrap();

    assert_eq!(rx.try_recv().unwrap(), (3, 10, 10, 10));
    assert_eq!(rx.try_recv().unwrap(), (3, 20, 20, 20));
    assert_eq!(rx.try_recv().unwrap(), (3, 30, 30, 30));
    assert!(rx.try_recv().is_err());
}

#[test]
fn duplicate_apply_does_not_run_after_apply() {
    let (tx, rx) = flume::bounded(4);
    let contime =
        contime::Contime::<SnapshotLanes, EventLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });
    let event = OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 };

    contime.apply_event(event.clone()).unwrap();
    contime.apply_event(event).unwrap();

    assert_eq!(rx.try_recv().unwrap(), (3, 10, 10, 10));
    assert!(rx.try_recv().is_err());
}

#[test]
fn query_materialization_does_not_run_after_apply() {
    let (tx, rx) = flume::bounded(4);
    let contime =
        contime::Contime::<SnapshotLanes, EventLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });

    contime.apply_event(OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 }).unwrap();
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
    let contime = contime::Contime::<SnapshotLanes, EventLanes, BlockingApplyTrace>::new_with_apply_context(
        1,
        100_000,
        BlockingApplyTrace { entered_tx, release_rx, applied: Arc::clone(&applied) },
    );

    contime.send_event(OnContextValueChanged { event_id: 10, time: 10, entity_id: 3, value: 10 }).unwrap();

    entered_rx.recv_timeout(std::time::Duration::from_secs(1)).unwrap();
    assert!(applied.lock().unwrap().is_empty());

    release_tx.send(()).unwrap();
    let snapshot = contime.query_at(11, &[3]).unwrap().pop().flatten().unwrap();
    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 11, value: 10 }));
}
