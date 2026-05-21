use contime::{AfterApplyEvents, ApplyBatch, ApplyEvents, Event, Snapshot, SnapshotEvent};

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
    applied: Vec<(u128, i64, usize, u64)>,
}

#[derive(Clone)]
struct ApplyTraceSender {
    tx: flume::Sender<(u128, i64, i32, i32)>,
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
        if let Some(event) = batch.events.last() {
            self.entity_id = event.entity_id;
            self.value = event.value;
        }
        self.time = batch.time;
    }
}

impl AfterApplyEvents<ApplyTrace> for ContextValueAt {
    fn after_apply_events(&self, _batch: ApplyBatch<'_, Self::Event>, context: &mut ApplyTrace) {
        context.applied.push((self.entity_id, self.time, self.value));
    }
}

impl AfterApplyEvents<ApplyTraceSender> for ContextValueAt {
    fn after_apply_events(&self, batch: ApplyBatch<'_, Self::Event>, context: &mut ApplyTraceSender) {
        let event = batch.events.last().expect("after apply should receive non-empty bucket");
        context.tx.send((event.entity_id, event.time, event.value, self.value)).unwrap();
    }
}

impl AfterApplyEvents<ApplyBatchTrace> for ContextValueAt {
    fn after_apply_events(&self, batch: ApplyBatch<'_, Self::Event>, context: &mut ApplyBatchTrace) {
        context.applied.push((batch.snapshot_id, batch.time, batch.events.len(), batch.bucket_revision));
    }
}

impl AfterApplyEvents for ContextValueAt {}

contime::contime! {
    mod context_contime;
    snapshots { ContextValueAt },
    OnContextValueChanged(OnContextValueChanged) => [ContextValueAt],
}

use context_contime::{EventLanes, SnapshotLanes};

#[test]
fn context_free_apply_still_mutates_snapshot() {
    let event = OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 };
    let mut snapshot = ContextValueAt::default();

    <ContextValueAt as ApplyEvents>::apply_events(
        &mut snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[event], bucket_revision: 0 },
    );

    assert_eq!(snapshot, ContextValueAt { entity_id: 3, time: 2, value: 4 });
}

#[test]
fn after_apply_receives_post_apply_snapshot_without_changing_snapshot_semantics() {
    let event = OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 };
    let mut snapshot = ContextValueAt::default();
    let mut context = ApplyTrace::default();

    <ContextValueAt as ApplyEvents>::apply_events(
        &mut snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[event.clone()], bucket_revision: 0 },
    );
    <ContextValueAt as AfterApplyEvents<ApplyTrace>>::after_apply_events(
        &snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[event], bucket_revision: 0 },
        &mut context,
    );

    assert_eq!(snapshot, ContextValueAt { entity_id: 3, time: 2, value: 4 });
    assert_eq!(context.applied, vec![(3, 2, 4)]);
}

#[test]
fn generated_lane_dispatch_passes_after_apply_to_concrete_event() {
    let event = EventLanes::OnContextValueChanged(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 });
    let mut snapshot = SnapshotLanes::ContextValueAt(ContextValueAt::default());
    let mut context = ApplyTrace::default();

    <SnapshotLanes as ApplyEvents>::apply_events(
        &mut snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[event], bucket_revision: 7 },
    );
    <SnapshotLanes as AfterApplyEvents<ApplyTrace>>::after_apply_events(
        &snapshot,
        ApplyBatch {
            snapshot_id: ContextValueAt::lane_id(3),
            time: 2,
            events: &[EventLanes::OnContextValueChanged(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 })],
            bucket_revision: 7,
        },
        &mut context,
    );

    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 2, value: 4 }));
    assert_eq!(context.applied, vec![(3, 2, 4)]);
}

#[test]
fn generated_lane_dispatch_passes_routed_snapshot_id_to_apply_batch() {
    let event = EventLanes::OnContextValueChanged(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 });
    let mut snapshot = SnapshotLanes::ContextValueAt(ContextValueAt::default());
    let mut context = ApplyBatchTrace::default();

    <SnapshotLanes as ApplyEvents>::apply_events(
        &mut snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[event.clone()], bucket_revision: 9 },
    );
    <SnapshotLanes as AfterApplyEvents<ApplyBatchTrace>>::after_apply_events(
        &snapshot,
        ApplyBatch { snapshot_id: ContextValueAt::lane_id(3), time: 2, events: &[event], bucket_revision: 9 },
        &mut context,
    );

    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 2, value: 4 }));
    assert_eq!(context.applied, vec![(3, 2, 1, 9)]);
}

#[test]
fn contime_workers_use_configured_apply_context() {
    let (tx, rx) = flume::bounded(1);
    let contime =
        contime::Contime::<SnapshotLanes, EventLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });

    contime.apply_event(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 }).unwrap();

    assert_eq!(rx.try_recv().unwrap(), (3, 2, 4, 4));
    let snapshot = contime.many_at(3, &[3]).unwrap().pop().flatten().unwrap();
    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 3, value: 4 }));
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
fn older_event_after_applied_schedule_runs_after_apply_for_future_bucket() {
    let (tx, rx) = flume::bounded(8);
    let contime =
        contime::Contime::<SnapshotLanes, EventLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });

    contime.apply_event(OnContextValueChanged { event_id: 1, time: 1, entity_id: 3, value: 1 }).unwrap();
    contime.schedule_event(OnContextValueChanged { event_id: 1001, time: 1001, entity_id: 3, value: 1001 }).unwrap();
    contime.advance_to(1100).expect("scheduled event should apply");

    assert_eq!(rx.try_recv().unwrap(), (3, 1, 1, 1));
    assert_eq!(rx.try_recv().unwrap(), (3, 1001, 1001, 1001));
    assert!(rx.try_recv().is_err());

    contime.apply_event(OnContextValueChanged { event_id: 876, time: 876, entity_id: 3, value: 876 }).unwrap();

    assert_eq!(rx.try_recv().unwrap(), (3, 876, 876, 876));
    assert_eq!(rx.try_recv().unwrap(), (3, 1001, 1001, 1001));
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

    let snapshot = contime.many_at(11, &[3]).unwrap().pop().flatten().unwrap();
    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 11, value: 10 }));
    assert!(rx.try_recv().is_err());
}
