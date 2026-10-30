use std::{cell::RefCell, rc::Rc};

use contime::ContimeModule;

struct Named {
    event_id: u128,
    time: i64,
    owner: u64,
    slot: u32,
    amount: i64,
}
impl contime::checkpoints::Event for Named {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl contime::EventId for Named {
    fn event_id(&self) -> u128 {
        self.event_id
    }
}

#[derive(Hash)]

struct Many {
    time: i64,
    targets: Vec<(u64, u32)>,
    amount: i64,
}
impl contime::checkpoints::Event for Many {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl contime::EventId for Many {
    fn event_id(&self) -> u128 {
        crate::test_id(std::any::type_name::<Self>(), self)
    }
}

#[derive(Clone, Default)]

struct Total {
    time: i64,
    owner: u64,
    slot: u32,
    total: i64,
}
impl contime::checkpoints::Snapshot for Total {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl Total {
    fn id_for(owner: &u64, slot: &u32) -> u128 {
        crate::test_id(std::any::type_name::<Self>(), &(owner, slot))
    }
}
impl contime::SnapshotId for Total {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.owner, &self.slot)
    }
}
impl contime::SnapshotApply for Total {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Named>(|event, emit| {
            emit(Total::id_for(&event.owner, &event.slot));
        });
        events.event::<Many>(|event, emit| {
            for target in &event.targets {
                emit(Total::id_for(&target.0, &target.1));
            }
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<Named>() {
            let mut snapshot = Self::default();
            {
                snapshot.owner = event.owner;
                snapshot.slot = event.slot;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        if let Some(event) = event.get::<Many>() {
            let mut snapshot = Self::default();
            for target in &(event.targets) {
                snapshot.owner = target.0;
                snapshot.slot = target.1;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        for event in batch.events::<Named>() {
            self.total += event.amount;
        }
        for event in batch.events::<Many>() {
            self.total += event.amount;
        }
    }
}

#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Named, Many], snapshots = [Total])]
struct Routed;

#[test]
fn compound_identities_and_multi_target_routes_preserve_explicit_event_identity() {
    let mut history = contime::manual::ManualConTime::<Routed>::new(contime::manual::ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(32).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    history
        .apply([
            RoutedEventLanes::from(Named { event_id: 1, time: -1, owner: 7, slot: 2, amount: 5 }),
            // Explicit identity takes precedence over different payload contents.
            Named { event_id: 1, time: -1, owner: 7, slot: 2, amount: 99 }.into(),
            Many { time: 0, targets: vec![(7, 2), (7, 3)], amount: 3 }.into(),
        ])
        .unwrap();
    let observed = Rc::new(RefCell::new(Vec::new()));
    let output = Rc::clone(&observed);
    history
        .query_at(0, [Total::id_for(&(7), &(2)), Total::id_for(&(7), &(3))], move |states| {
            let mut values = states
                .into_iter()
                .map(|state| {
                    let RoutedSnapshotLanes::Total(total) = *state;
                    (total.owner, total.slot, total.total)
                })
                .collect::<Vec<_>>();
            values.sort_unstable();
            *output.borrow_mut() = values;
        })
        .unwrap();
    for _ in 0..1000 {
        if !history.process().unwrap() {
            break;
        }
    }
    assert_eq!(*observed.borrow(), [(7, 2, 8), (7, 3, 3)]);
    assert!(history.take_rejections().is_empty());
}

#[derive(Hash)]

struct Slots {
    time: i64,
    owner: u64,
    slots: Vec<u64>,
}
impl contime::checkpoints::Event for Slots {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl contime::EventId for Slots {
    fn event_id(&self) -> u128 {
        crate::test_id(std::any::type_name::<Self>(), self)
    }
}

#[derive(Clone, Default)]

struct ScalarSlot {
    time: i64,
    slot: u64,
}
impl contime::checkpoints::Snapshot for ScalarSlot {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ScalarSlot {
    fn id_for(slot: &u64) -> u128 {
        crate::test_id(std::any::type_name::<Self>(), &(slot,))
    }
}
impl contime::SnapshotId for ScalarSlot {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.slot)
    }
}
impl contime::SnapshotApply for ScalarSlot {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Slots>(|event, emit| {
            for slot in &event.slots {
                emit(ScalarSlot::id_for(slot));
            }
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<Slots>() {
            let mut snapshot = Self::default();
            for slot in &(event.slots) {
                snapshot.slot = *slot;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        for _event in batch.events::<Slots>() {}
    }
}

#[derive(Clone, Default)]

struct OwnedSlot {
    time: i64,
    owner: u64,
    slot: u64,
}
impl contime::checkpoints::Snapshot for OwnedSlot {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl OwnedSlot {
    fn id_for(owner: &u64, slot: &u64) -> u128 {
        crate::test_id(std::any::type_name::<Self>(), &(owner, slot))
    }
}
impl contime::SnapshotId for OwnedSlot {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.owner, &self.slot)
    }
}
impl contime::SnapshotApply for OwnedSlot {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Slots>(|event, emit| {
            for slot in &event.slots {
                emit(OwnedSlot::id_for(&event.owner, slot));
            }
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<Slots>() {
            let mut snapshot = Self::default();
            for slot in &(event.slots) {
                snapshot.owner = event.owner;
                snapshot.slot = *slot;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        for _event in batch.events::<Slots>() {}
    }
}

#[test]
fn scalar_each_routes_clone_the_complete_identity_expression() {
    let event = Slots { time: -5, owner: 7, slots: vec![2, 3] };
    let mut ids = Vec::new();
    contime::module::EventSet::<ScalarSlot>::new().snapshot_ids(contime::EventRef::new(&event), &mut |id| ids.push(id));
    assert_eq!(ids, [ScalarSlot::id_for(&2), ScalarSlot::id_for(&3)]);
    for (id, expected) in ids.into_iter().zip([2, 3]) {
        assert_eq!(contime::module::EventSet::<ScalarSlot>::new().create(id, contime::EventRef::new(&event)).unwrap().slot, expected);
    }
    assert!(contime::module::EventSet::<ScalarSlot>::new().create(ScalarSlot::id_for(&99), contime::EventRef::new(&event)).is_none());
}

#[test]
fn compound_each_routes_can_read_event_fields_and_item_values() {
    let event = Slots { time: -5, owner: 7, slots: vec![2, 3] };
    let mut ids = Vec::new();
    contime::module::EventSet::<OwnedSlot>::new().snapshot_ids(contime::EventRef::new(&event), &mut |id| ids.push(id));
    assert_eq!(ids, [OwnedSlot::id_for(&7, &2), OwnedSlot::id_for(&7, &3)]);
    for (id, expected) in ids.into_iter().zip([2, 3]) {
        let state = contime::module::EventSet::<OwnedSlot>::new().create(id, contime::EventRef::new(&event)).unwrap();
        assert_eq!((state.owner, state.slot), (7, expected));
    }
    assert!(contime::module::EventSet::<OwnedSlot>::new().create(OwnedSlot::id_for(&8, &2), contime::EventRef::new(&event)).is_none());
}
