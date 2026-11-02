//! Derived registration uses the same builder as handwritten definitions.
use contime::prelude::*;

#[derive(ContimeEvent)]
#[contime(id = |event| event.id)]
struct Added {
    id: u128,
    #[contime(time)]
    at: i64,
    targets: Vec<u128>,
    value: i64,
}
#[derive(ContimeEvent)]
#[contime(id = |event| event.id)]
struct Multiplied {
    id: u128,
    #[contime(time)]
    at: i64,
    target: u128,
    value: i64,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |snapshot| snapshot.owner)]
#[contime(event = Added, id = added_ids)]
#[contime(id = multiplied_ids, event = Multiplied)]
struct Total {
    #[contime(time)]
    at: i64,
    owner: u128,
    value: i64,
}
fn added_ids(event: &Added, emit: &mut dyn FnMut(u128)) {
    for id in &event.targets {
        emit(*id);
    }
}
fn multiplied_ids(event: &Multiplied, emit: &mut dyn FnMut(u128)) {
    emit(event.target);
}
impl contime::SnapshotCreate for Total {
    fn create(owner: u128, _: EventRef<'_>) -> Self {
        Self { owner, ..Self::default() }
    }
}
impl SnapshotApply for Total {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Added>() {
            self.value += event.value;
        }
        for event in batch.events::<Multiplied>() {
            self.value *= event.value;
        }
    }
}
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added, Multiplied], snapshots = [Total])]
struct App;

#[test]
fn repeated_attributes_route_zero_one_and_many_destinations_and_replay_late_inputs() {
    let c = ConTime::<App>::start(crate::config()).unwrap();
    c.apply([
        Added { id: 1, at: 10, targets: vec![7, 8], value: 2 }.into(),
        Multiplied { id: 2, at: 10, target: 7, value: 3 }.into(),
        Added { id: 3, at: 10, targets: vec![], value: 99 }.into(),
    ])
    .unwrap();
    c.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let values = |states: Vec<Box<AppSnapshotLanes>>| {
        let mut v: Vec<_> = states
            .iter()
            .map(|s| {
                let AppSnapshotLanes::Total(s) = &**s;
                (s.owner, s.value)
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(values(c.query_at(10, [7, 8]).unwrap()), [(7, 6), (8, 2)]);
    c.apply([Added { id: 4, at: -10, targets: vec![7, 8], value: 1 }.into()]).unwrap();
    c.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(values(c.query_at(10, [7, 8]).unwrap()), [(7, 9), (8, 3)]);
    assert!(c.errors().is_empty());
    c.shutdown();
}

// Registration-only derives also work with handwritten timestamps and IDs.
#[derive(Clone, ContimeSnapshot)]
#[contime(event = Added, id = added_ids)]
struct Local<T: Clone + Default + 'static> {
    at: i64,
    owner: u128,
    payload: T,
    value: i64,
}
impl<T: Clone + Default + 'static> contime::checkpoints::Snapshot for Local<T> {
    type Time = i64;
    fn time(&self) -> &i64 {
        &self.at
    }
    fn set_time(&mut self, at: i64) {
        self.at = at;
    }
}
impl<T: Clone + Default + 'static> SnapshotId for Local<T> {
    fn snapshot_id(&self) -> u128 {
        self.owner
    }
}
impl<T: Clone + Default + 'static> contime::SnapshotCreate for Local<T> {
    fn create(owner: u128, _: EventRef<'_>) -> Self {
        Self { at: 0, owner, payload: T::default(), value: 0 }
    }
}
impl<T: Clone + Default + 'static> SnapshotApply for Local<T> {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Added>() {
            self.value += event.value;
        }
    }
}
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added], snapshots = [Local = Local<std::rc::Rc<std::cell::Cell<i64>>>])]
struct LocalApp;

#[test]
fn registration_only_derive_preserves_generics_manual_time_and_ids_and_local_data() {
    let mut c = ManualConTime::<LocalApp>::new(ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(32).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    c.apply([Added { id: 1, at: -5, targets: vec![7], value: 2 }.into()]).unwrap();
    let observed = std::rc::Rc::new(std::cell::Cell::new(false));
    let output = observed.clone();
    c.query_at(-5, [7], move |states| {
        let LocalAppSnapshotLanes::Local(state) = &*states[0];
        assert_eq!(state.value, 2);
        assert_eq!(state.at, -5);
        assert_eq!(state.snapshot_id(), 7);
        assert_eq!(state.payload.get(), 0);
        output.set(true);
    })
    .unwrap();
    while c.process().unwrap() {}
    assert!(observed.get());
}
