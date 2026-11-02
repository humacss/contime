//! Snapshot metadata supplies module lanes; filters belong to snapshot instances.
use contime::prelude::*;
#[derive(ContimeEvent)]
struct Added {
    #[contime(id)]
    id: u128,
    owner: u128,
    #[contime(time)]
    at: i64,
    amount: i64,
}
#[derive(ContimeEvent)]
struct Multiplied {
    #[contime(id)]
    id: u128,
    owner: u128,
    #[contime(time)]
    at: i64,
    factor: i64,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.owner, map = |id| id * 2)]
#[contime(events = [Added, Multiplied])]
struct Counter {
    owner: u128,
    #[contime(time)]
    at: i64,
    value: i64,
}
impl SnapshotApply for Counter {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for e in batch.events::<Added>() {
            self.value += e.amount;
        }
        for e in batch.events::<Multiplied>() {
            self.value *= e.factor;
        }
    }
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.owner, map = |id| id * 2 + 1)]
#[contime(events = [Added, Multiplied])]
struct Other {
    owner: u128,
    #[contime(time)]
    at: i64,
    value: i64,
}
impl SnapshotApply for Other {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for e in batch.events::<Added>() {
            self.value += e.amount;
        }
        for e in batch.events::<Multiplied>() {
            self.value *= e.factor;
        }
    }
}
#[derive(ContimeModule)]
#[contime(snapshot = Counter, events = [Added])]
#[allow(dead_code)]
struct Restricted;
#[derive(ContimeModule)]
#[contime(snapshots = [Other])]
#[contime(modules = [Restricted])]
struct App;
#[derive(ContimeModule)]
#[contime(snapshots = [Counter])]
struct Full;
#[test]
fn subsets_stay_local_when_other_snapshots_include_the_excluded_event() {
    let c = ConTime::<App>::start(crate::config()).unwrap();
    c.apply([Added { id: 1, owner: 7, at: -10, amount: 2 }.into(), Multiplied { id: 2, owner: 7, at: 10, factor: 3 }.into()]).unwrap();
    c.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let states = c.query_at(10, [14, 15]).unwrap();
    assert_eq!(states.len(), 2);
    for state in states {
        match *state {
            AppSnapshotLanes::Counter(s) => assert_eq!(s.value, 2),
            AppSnapshotLanes::Other(s) => assert_eq!(s.value, 6),
        }
    }
    assert!(c.errors().is_empty());
    c.shutdown();
    // Restricting Counter in one module does not modify its global definition.
    let full = ConTime::<Full>::start(crate::config()).unwrap();
    full.apply([Added { id: 1, owner: 7, at: -10, amount: 2 }.into(), Multiplied { id: 2, owner: 7, at: 10, factor: 3 }.into()]).unwrap();
    full.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let states = full.query_at(10, [14]).unwrap();
    let FullSnapshotLanes::Counter(s) = &*states[0];
    assert_eq!(s.value, 6);
    full.shutdown();
}

#[derive(ContimeModule)]
#[contime(snapshot = Counter, events = [Added])]
#[allow(dead_code)]
struct Left;
#[derive(ContimeModule)]
#[contime(snapshot = Counter, events = [Multiplied])]
#[allow(dead_code)]
struct Right;
#[derive(ContimeModule)]
#[contime(modules = [Left, Right, Left])]
struct Combined;
#[derive(ContimeModule)]
#[contime(snapshots = [Counter])]
#[contime(modules = [Left])]
struct FullDominates;
#[test]
fn repeated_snapshot_contributions_union_subsets_and_full_registration_dominates() {
    let c = ConTime::<Combined>::start(crate::config()).unwrap();
    c.apply([Added { id: 1, owner: 7, at: -10, amount: 2 }.into(), Multiplied { id: 2, owner: 7, at: 10, factor: 3 }.into()]).unwrap();
    c.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let states = c.query_at(10, [14]).unwrap();
    assert_eq!(states.len(), 1);
    let CombinedSnapshotLanes::Counter(s) = &*states[0];
    assert_eq!(s.value, 6);
    c.shutdown();
    let c = ConTime::<FullDominates>::start(crate::config()).unwrap();
    c.apply([Added { id: 1, owner: 7, at: -10, amount: 2 }.into(), Multiplied { id: 2, owner: 7, at: 10, factor: 3 }.into()]).unwrap();
    c.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let states = c.query_at(10, [14]).unwrap();
    let FullDominatesSnapshotLanes::Counter(s) = &*states[0];
    assert_eq!(s.value, 6);
    c.shutdown();
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.owner)]
#[contime(events = [Added])]
struct Generic<T: Clone + Default + 'static> {
    owner: u128,
    #[contime(time)]
    at: i64,
    payload: T,
}
impl<T: Clone + Default + 'static> SnapshotApply for Generic<T> {}
#[derive(ContimeModule)]
#[contime(snapshots = [Numeric = Generic<u64>, Local = Generic<std::rc::Rc<std::cell::Cell<u64>>>])]
struct GenericApp;
#[derive(ContimeModule)]
#[contime(snapshots = [Local = Generic<std::rc::Rc<std::cell::Cell<u64>>>])]
struct LocalApp;
#[test]
fn generic_metadata_keeps_distinct_instantiations_and_supports_non_send_manual_state() {
    let _ = GenericApp;
    let _: GenericAppSnapshotLanes = Generic::<u64>::default().into();
    let _: GenericAppSnapshotLanes = Generic::<std::rc::Rc<std::cell::Cell<u64>>>::default().into();
    let mut c = ManualConTime::<LocalApp>::new(ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(16).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    c.apply([Added { id: 1, owner: 7, at: -5, amount: 2 }.into()]).unwrap();
    let done = std::rc::Rc::new(std::cell::Cell::new(false));
    let output = done.clone();
    c.query_at(-5, [7], move |states| {
        let LocalAppSnapshotLanes::Local(s) = &*states[0];
        assert_eq!(s.owner, 7);
        assert_eq!(s.at, -5);
        assert_eq!(s.payload.get(), 0);
        output.set(true);
    })
    .unwrap();
    while c.process().unwrap() {}
    assert!(done.get());
}

#[derive(ContimeModule)]
#[contime(snapshot = Counter, events = [])]
struct NoTraffic;
#[test]
fn an_empty_snapshot_subset_is_valid_without_inventing_a_time_type() {
    let c = ConTime::<NoTraffic>::start(crate::config()).unwrap();
    assert!(c.query_at(0, [14]).unwrap().is_empty());
    c.shutdown();
}

#[derive(ContimeEvent)]
struct Pair<A, B> {
    #[contime(id)]
    id: u128,
    owner: u128,
    #[contime(time)]
    at: i64,
    _marker: std::marker::PhantomData<(A, B)>,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(events=[Pair<T,&'a str>])]
struct LifeState<'a: 'static, T: Clone + Default + 'static> {
    #[contime(id)]
    owner: u128,
    #[contime(time)]
    at: i64,
    _marker: std::marker::PhantomData<(&'a (), T)>,
}
impl<'a: 'static, T: Clone + Default + 'static> SnapshotApply for LifeState<'a, T> {}
#[derive(ContimeModule)]
#[contime(snapshots=[LifeState<'static,u64>])]
struct Borrowed;
#[test]
fn nested_lifetime_arguments_produce_well_formed_concrete_event_aliases() {
    let _ = Borrowed;
    let _: BorrowedEventLanes = Pair::<u64, &'static str> { id: 1, owner: 7, at: 0, _marker: std::marker::PhantomData }.into();
}
