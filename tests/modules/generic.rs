use contime::EventId;
use std::{cell::RefCell, hash::Hash, ops::AddAssign, rc::Rc};

use contime::ContimeModule;

#[derive(Hash)]

struct GenericEvent<T: Hash + 'static> {
    time: i64,
    counter_id: u128,
    amount: T,
}
impl<T: Hash + 'static> ::contime::checkpoints::Event for GenericEvent<T>
where
    i64: ::core::clone::Clone + ::core::cmp::PartialEq,
{
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl<T: Hash + 'static> contime::EventId for GenericEvent<T> {
    fn event_id(&self) -> u128 {
        crate::test_id(std::any::type_name::<Self>(), self)
    }
}

struct GenericSnapshot<T>
where
    T: Clone + Default + Hash + AddAssign + 'static,
{
    time: i64,
    counter_id: u128,
    value: T,
}
impl<T> ::core::clone::Clone for GenericSnapshot<T>
where
    T: Clone + Default + Hash + AddAssign + 'static,
    i64: ::core::clone::Clone,
    u128: ::core::clone::Clone,
    T: ::core::clone::Clone,
{
    fn clone(&self) -> Self {
        Self { time: self.time, counter_id: self.counter_id, value: self.value.clone() }
    }
}
impl<T> ::core::default::Default for GenericSnapshot<T>
where
    T: Clone + Default + Hash + AddAssign + 'static,
    i64: ::core::default::Default,
    u128: ::core::default::Default,
    T: ::core::default::Default,
{
    fn default() -> Self {
        Self {
            time: ::core::default::Default::default(),
            counter_id: ::core::default::Default::default(),
            value: ::core::default::Default::default(),
        }
    }
}
impl<T> ::contime::checkpoints::Snapshot for GenericSnapshot<T>
where
    T: Clone + Default + Hash + AddAssign + 'static,
    Self: ::core::clone::Clone + ::core::default::Default,
    i64: ::core::clone::Clone + ::core::default::Default + ::core::cmp::Ord,
{
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl<T> GenericSnapshot<T>
where
    T: Clone + Default + Hash + AddAssign + 'static,
{
    fn id_for(counter_id: &u128) -> u128 {
        crate::test_id(std::any::type_name::<Self>(), &(counter_id,))
    }
}
impl<T> contime::SnapshotId for GenericSnapshot<T>
where
    T: Clone + Default + Hash + AddAssign + 'static,
{
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.counter_id)
    }
}
impl<T> contime::SnapshotApply for GenericSnapshot<T>
where
    T: Clone + Default + Hash + AddAssign + 'static,
{
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<GenericEvent<T>>(|event, emit| {
            emit(<GenericSnapshot<T>>::id_for(&event.counter_id));
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<GenericEvent<T>>() {
            let mut snapshot = Self::default();
            {
                snapshot.counter_id = event.counter_id;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        for event in batch.events::<GenericEvent<T>>() {
            self.value += event.amount.clone();
        }
    }
}

// Different module markers register the same concrete generic types. Their
// shared lanes must be included once, including the root's direct registration.
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [GenericEvent<i64>], snapshots = [GenericSnapshot<i64>])]
#[allow(dead_code)]
struct First;

#[derive(ContimeModule)]
#[contime_module(time = i64, events = [GenericEvent<i64>], snapshots = [GenericSnapshot<i64>])]
#[allow(dead_code)]
struct Second;

#[derive(ContimeModule)]
#[contime_module(time = i64, events = [GenericEvent<i64>], snapshots = [GenericSnapshot<i64>], modules = [First, Second])]
struct GenericApplication;

#[test]
fn concrete_generic_lanes_propagate_bounds_and_deduplicate_repeated_registrations() {
    let first = GenericEvent { time: -2, counter_id: 7, amount: 3_i64 };
    let duplicate = GenericEvent { time: -2, counter_id: 7, amount: 3_i64 };
    assert_eq!(first.event_id(), duplicate.event_id());
    assert_eq!(GenericSnapshot::<i64>::default().clone().value, 0);

    let mut history = contime::manual::ManualConTime::<GenericApplication>::new(contime::manual::ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(16).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    history
        .apply([GenericApplicationEventLanes::from(first), duplicate.into(), GenericEvent { time: 0, counter_id: 7, amount: 5_i64 }.into()])
        .unwrap();
    let observed = Rc::new(RefCell::new(None));
    let output = Rc::clone(&observed);
    history
        .query_at(0, [GenericSnapshot::<i64>::id_for(&(7))], move |states| {
            assert_eq!(states.len(), 1);
            let GenericApplicationSnapshotLanes::GenericSnapshot(snapshot) = &*states[0];
            *output.borrow_mut() = Some((snapshot.counter_id, snapshot.value));
        })
        .unwrap();
    for _ in 0..1000 {
        if !history.process().unwrap() {
            break;
        }
    }
    assert_eq!(*observed.borrow(), Some((7, 8)));
    assert!(history.take_rejections().is_empty());
}
