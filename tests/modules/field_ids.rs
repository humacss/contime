//! Identity fields drive routing and creation without application-specific dispatch.
use contime::prelude::*;

#[derive(ContimeEvent)]
struct Added {
    #[contime(id)]
    event_id: u128,
    list_id: u128,
    #[contime(time)]
    at: i64,
    value: char,
}
#[derive(ContimeEvent)]
#[contime(id = |value| value.event_id)]
struct Removed {
    event_id: u128,
    list_id: u128,
    #[contime(time)]
    at: i64,
}
#[derive(ContimeEvent)]
struct Forwarded {
    #[contime(id)]
    event_id: u128,
    destination: u128,
    #[contime(time)]
    at: i64,
    value: char,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(events = [Added, Removed])]
#[contime(event = Forwarded, id = |value| value.destination)]
struct List {
    #[contime(id)]
    list_id: u128,
    #[contime(time)]
    at: i64,
    values: Vec<char>,
}
impl SnapshotApply for List {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for value in batch.events::<Added>() {
            self.values.push(value.value);
        }
        for value in batch.events::<Forwarded>() {
            self.values.push(value.value);
        }
        for _ in batch.events::<Removed>() {
            self.values.pop();
        }
    }
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |value| value.list_id, map = |id| id + 1000)]
#[contime(event = Added)]
struct Untouched {
    list_id: u128,
    #[contime(time)]
    at: i64,
    value: u64,
}
impl SnapshotApply for Untouched {}
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added, Removed, Forwarded], snapshots = [List, Untouched])]
struct App;
#[test]
fn automatic_creation_uses_inherited_and_renamed_fields_and_replays_negative_events() {
    let c = ConTime::<App>::start(crate::config()).unwrap();
    c.apply([Added { event_id: 1, list_id: 7, at: 10, value: 'c' }.into()]).unwrap();
    c.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    c.apply([
        Added { event_id: 2, list_id: 7, at: -10, value: 'a' }.into(),
        Forwarded { event_id: 3, destination: 7, at: 0, value: 'b' }.into(),
    ])
    .unwrap();
    c.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let states = c.query_at(10, [7, 1007]).unwrap();
    assert_eq!(states.len(), 2);
    for state in states {
        match *state {
            AppSnapshotLanes::List(s) => {
                assert_eq!(s.list_id, 7);
                assert_eq!(s.values, ['a', 'b', 'c']);
            }
            AppSnapshotLanes::Untouched(s) => {
                assert_eq!(s.list_id, 7);
                assert_eq!(s.value, 0);
                assert_eq!(s.at, 10);
            }
        }
    }
    assert!(c.errors().is_empty());
    c.shutdown();
}

#[derive(ContimeEvent)]
#[contime(id = |v| (v.client_id, v.sequence), map = |(client, sequence)| (u128::from(client) << 64) | u128::from(sequence))]
struct Named {
    client_id: u64,
    sequence: u64,
    owner: String,
    slot: u128,
    #[contime(time)]
    at: i64,
}
#[derive(ContimeEvent)]
struct Renamed {
    #[contime(id)]
    id: u128,
    name: String,
    index: u128,
    #[contime(time)]
    at: i64,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| (&v.owner, v.slot), map = |(owner, slot)| owner.len() as u128 * 100 + slot)]
#[contime(event = Named, map = |(owner, slot)| slot + owner.len() as u128 * 100)]
#[contime(event = Renamed, id = |v| (&v.name, v.index), map = |(name, index)| name.len() as u128 * 100 + index)]
struct NamedState {
    owner: String,
    slot: u128,
    #[contime(time)]
    at: i64,
}
impl SnapshotApply for NamedState {}
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Named, Renamed], snapshots = [NamedState])]
struct Names;
#[test]
fn maps_support_borrowed_composite_fields_and_event_overrides_without_moving_payloads() {
    let value = Named { client_id: 3, sequence: 5, owner: "alice".into(), slot: 7, at: -1 };
    assert_eq!(value.event_id(), (3u128 << 64) | 5);
    let c = ConTime::<Names>::start(crate::config()).unwrap();
    c.apply([value.into(), Renamed { id: 99, name: "bob".into(), index: 2, at: 0 }.into()]).unwrap();
    c.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    for (id, name, slot) in [(507, "alice", 7), (302, "bob", 2)] {
        let states = c.query_at(0, [id]).unwrap();
        assert_eq!(states.len(), 1);
        let NamesSnapshotLanes::NamedState(s) = &*states[0];
        assert_eq!(s.owner, name);
        assert_eq!(s.slot, slot);
        assert_eq!(s.snapshot_id(), id);
    }
    assert!(c.errors().is_empty());
    c.shutdown();
}

#[derive(Default)]
struct CountedKey {
    id: u128,
    clones: std::rc::Rc<std::cell::Cell<usize>>,
}
impl Clone for CountedKey {
    fn clone(&self) -> Self {
        self.clones.set(self.clones.get() + 1);
        Self { id: self.id, clones: self.clones.clone() }
    }
}
#[derive(ContimeEvent)]
struct KeyedEvent {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    at: i64,
    key: CountedKey,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| &v.key, map = |key| key.id)]
#[contime(events = [KeyedEvent])]
struct KeyedState {
    key: CountedKey,
    #[contime(time)]
    at: i64,
}
impl SnapshotApply for KeyedState {}
#[test]
fn borrowed_id_routing_does_not_clone_fields_but_creation_copies_identity_once() {
    let clones = std::rc::Rc::new(std::cell::Cell::new(0));
    let event = KeyedEvent { id: 1, at: 0, key: CountedKey { id: 7, clones: clones.clone() } };
    let registration = contime::module::EventSet::<KeyedState>::new();
    for _ in 0..10 {
        let mut ids = Vec::new();
        registration.snapshot_ids(EventRef::new(&event), &mut |id| ids.push(id));
        assert_eq!(ids, [7]);
    }
    assert_eq!(clones.get(), 0);
    let snapshot = registration.create(7, EventRef::new(&event)).unwrap();
    assert_eq!(snapshot.snapshot_id(), 7);
    assert_eq!(clones.get(), 1);
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.list_id)]
#[contime(events = [Added])]
struct GenericState<T: Clone + Default + 'static> {
    list_id: u128,
    #[contime(time)]
    at: i64,
    payload: T,
}
impl<T: Clone + Default + 'static> SnapshotApply for GenericState<T> {}
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added], snapshots = [State = GenericState<std::rc::Rc<std::cell::Cell<u32>>>])]
struct LocalFields;
#[test]
fn inferred_creation_preserves_generic_bounds_and_manual_non_send_state() {
    let mut c = ManualConTime::<LocalFields>::new(ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(32).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    c.apply([Added { event_id: 1, list_id: 7, at: -5, value: 'a' }.into()]).unwrap();
    let observed = std::rc::Rc::new(std::cell::Cell::new(false));
    let output = observed.clone();
    c.query_at(-5, [7], move |states| {
        let LocalFieldsSnapshotLanes::State(s) = &*states[0];
        assert_eq!(s.list_id, 7);
        assert_eq!(s.at, -5);
        assert_eq!(s.payload.get(), 0);
        output.set(true);
    })
    .unwrap();
    while c.process().unwrap() {}
    assert!(observed.get());
}

// An ID-only derive does not force timestamp, Clone or Default implementations.
#[derive(ContimeSnapshot)]
struct IdentityOnly(#[contime(id)] u128);
#[test]
fn snapshot_id_field_helper_also_supports_id_only_tuple_types() {
    assert_eq!(IdentityOnly(42).snapshot_id(), 42);
}
