use contime::{ApplyEvent, Event, Snapshot};

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

#[derive(Clone)]
struct ApplyTraceSender {
    tx: flume::Sender<(u128, i64, i32)>,
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

impl ApplyEvent<ContextValueAt> for OnContextValueChanged {
    fn snapshot_id(&self) -> u128 {
        ContextValueAt::lane_id(self.entity_id)
    }

    fn apply_to(&self, snapshot: &mut ContextValueAt) {
        snapshot.entity_id = self.entity_id;
        snapshot.time = self.time;
        snapshot.value = self.value;
    }
}

impl ApplyEvent<ContextValueAt, ApplyTrace> for OnContextValueChanged {
    fn snapshot_id(&self) -> u128 {
        ContextValueAt::lane_id(self.entity_id)
    }

    fn apply_to(&self, snapshot: &mut ContextValueAt) {
        <Self as ApplyEvent<ContextValueAt>>::apply_to(self, snapshot);
    }

    fn apply_to_with_context(&self, snapshot: &mut ContextValueAt, context: &mut ApplyTrace) {
        <Self as ApplyEvent<ContextValueAt>>::apply_to(self, snapshot);
        context.applied.push((self.entity_id, self.time, self.value));
    }
}

impl ApplyEvent<ContextValueAt, ApplyTraceSender> for OnContextValueChanged {
    fn snapshot_id(&self) -> u128 {
        ContextValueAt::lane_id(self.entity_id)
    }

    fn apply_to(&self, snapshot: &mut ContextValueAt) {
        <Self as ApplyEvent<ContextValueAt>>::apply_to(self, snapshot);
    }

    fn apply_to_with_context(&self, snapshot: &mut ContextValueAt, context: &mut ApplyTraceSender) {
        <Self as ApplyEvent<ContextValueAt>>::apply_to(self, snapshot);
        context.tx.send((self.entity_id, self.time, self.value)).unwrap();
    }
}

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

    <OnContextValueChanged as ApplyEvent<ContextValueAt>>::apply_to(&event, &mut snapshot);

    assert_eq!(snapshot, ContextValueAt { entity_id: 3, time: 2, value: 4 });
}

#[test]
fn context_aware_apply_receives_context_without_changing_snapshot_semantics() {
    let event = OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 };
    let mut snapshot = ContextValueAt::default();
    let mut context = ApplyTrace::default();

    event.apply_to_with_context(&mut snapshot, &mut context);

    assert_eq!(snapshot, ContextValueAt { entity_id: 3, time: 2, value: 4 });
    assert_eq!(context.applied, vec![(3, 2, 4)]);
}

#[test]
fn generated_lane_dispatch_passes_context_to_concrete_apply() {
    let event = EventLanes::OnContextValueChanged(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 });
    let mut snapshot = SnapshotLanes::ContextValueAt(ContextValueAt::default());
    let mut context = ApplyTrace::default();

    event.apply_to_with_context(&mut snapshot, &mut context);

    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 2, value: 4 }));
    assert_eq!(context.applied, vec![(3, 2, 4)]);
}

#[test]
fn contime_workers_use_configured_apply_context() {
    let (tx, rx) = flume::bounded(1);
    let contime =
        contime::Contime::<SnapshotLanes, EventLanes, ApplyTraceSender>::new_with_apply_context(1, 100_000, ApplyTraceSender { tx });

    contime.apply_event(OnContextValueChanged { event_id: 1, time: 2, entity_id: 3, value: 4 }).unwrap();

    assert_eq!(rx.try_recv().unwrap(), (3, 2, 4));
    let snapshot = contime.many_at(3, &[3]).unwrap().pop().flatten().unwrap();
    assert_eq!(snapshot, SnapshotLanes::ContextValueAt(ContextValueAt { entity_id: 3, time: 3, value: 4 }));
}
