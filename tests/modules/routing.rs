use std::{cell::RefCell, rc::Rc};

use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};

#[derive(ContimeEvent)]
#[contime_event(id = self.event_id)]
struct Named {
    event_id: u128,
    time: i64,
    owner: u64,
    slot: u32,
    amount: i64,
}

#[derive(ContimeEvent)]
struct Many {
    time: i64,
    targets: Vec<(u64, u32)>,
    amount: i64,
}

#[derive(ContimeSnapshot)]
#[contime_snapshot(
    id = [self.owner, self.slot],
    events = [Named => [self.owner, self.slot], Many => each(self.targets, |target| [target.0, target.1])],
    apply = {
        for event in batch.events {
            self.total += match event {
                TotalEvent::Named(event) => event.amount,
                TotalEvent::Many(event) => event.amount,
            };
        }
    }
)]
struct Total {
    time: i64,
    owner: u64,
    slot: u32,
    total: i64,
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
        .query_at(0, [Total::snapshot_id(7, 2), Total::snapshot_id(7, 3)], move |states| {
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

#[derive(ContimeEvent)]
struct Slots {
    time: i64,
    owner: u64,
    slots: Vec<u64>,
}

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = [self.slot], events = [Slots => each(self.slots, |slot| [*slot])], apply = {
    for _event in batch.events {}
})]
struct ScalarSlot {
    time: i64,
    slot: u64,
}

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = [self.owner, self.slot], events = [Slots => each(self.slots, |slot| [self.owner, *slot])], apply = {
    for _event in batch.events {}
})]
struct OwnedSlot {
    time: i64,
    owner: u64,
    slot: u64,
}

#[test]
fn scalar_each_routes_clone_the_complete_identity_expression() {
    use contime::module::Route;
    let event = Slots { time: -5, owner: 7, slots: vec![2, 3] };
    let mut ids = Vec::new();
    ScalarSlot::snapshot_ids(&event, &mut |id| ids.push(id));
    assert_eq!(ids, [ScalarSlot::snapshot_id(2), ScalarSlot::snapshot_id(3)]);
    for (id, expected) in ids.into_iter().zip([2, 3]) {
        assert_eq!(ScalarSlot::create(id, &event).unwrap().slot, expected);
    }
    assert!(ScalarSlot::create(ScalarSlot::snapshot_id(99), &event).is_none());
}

#[test]
fn compound_each_routes_can_read_event_fields_and_item_values() {
    use contime::module::Route;
    let event = Slots { time: -5, owner: 7, slots: vec![2, 3] };
    let mut ids = Vec::new();
    OwnedSlot::snapshot_ids(&event, &mut |id| ids.push(id));
    assert_eq!(ids, [OwnedSlot::snapshot_id(7, 2), OwnedSlot::snapshot_id(7, 3)]);
    for (id, expected) in ids.into_iter().zip([2, 3]) {
        let state = OwnedSlot::create(id, &event).unwrap();
        assert_eq!((state.owner, state.slot), (7, expected));
    }
    assert!(OwnedSlot::create(OwnedSlot::snapshot_id(8, 2), &event).is_none());
}
