use std::{cell::RefCell, hash::Hash, ops::AddAssign, rc::Rc};

use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};

#[derive(ContimeEvent)]
struct GenericEvent<T: Hash + 'static> {
    time: i64,
    counter_id: u128,
    amount: T,
}

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = [self.counter_id], events = [GenericEvent<T>], apply = {
    for event in batch.events {
        let GenericSnapshotEvent::GenericEvent(event) = event;
        self.value += event.amount.clone();
    }
})]
struct GenericSnapshot<T>
where
    T: Clone + Default + Hash + AddAssign + 'static,
{
    time: i64,
    counter_id: u128,
    value: T,
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
        .query_at(0, [GenericSnapshot::<i64>::snapshot_id(7)], move |states| {
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
