use contime::prelude::*;

struct Added {
    time: i64,
    id: u128,
    value: i64,
}
impl contime::checkpoints::Event for Added {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

struct Cleared {
    time: i64,
    id: u128,
}
impl contime::checkpoints::Event for Cleared {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl EventId for Added {
    fn event_id(&self) -> u128 {
        self.id
    }
}
impl EventId for Cleared {
    fn event_id(&self) -> u128 {
        self.id
    }
}

#[derive(Clone, Default)]
struct State {
    time: i64,
    id: u128,
    values: Vec<i64>,
    batches: Vec<(i64, u64)>,
}
impl contime::checkpoints::Snapshot for State {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl SnapshotId for State {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}
impl SnapshotApply for State {
    fn define(events: &mut EventSetBuilder<Self>) {
        events
            .event::<Added>(|_event, emit| {
                emit(7);
            })
            .event::<Cleared>(|_event, emit| {
                emit(7);
            });
    }
    fn create(id: u128, _event: EventRef<'_>) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        self.batches.push((*batch.time(), batch.history_event_count()));
        let additions = batch.events::<Added>().map(|e| e.value).collect::<Vec<_>>();
        assert_eq!(batch.events::<Added>().map(|e| e.value).collect::<Vec<_>>(), additions);
        for _ in batch.events::<Cleared>() {
            self.values.clear();
        }
        for event in batch.events::<Added>() {
            self.values.push(event.value);
        }
    }
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added,Cleared],snapshots=[State])]
struct App;
#[test]
fn type_phases_override_cross_type_ids_but_keep_timestamps_and_replay_consistent() {
    let contime = ConTime::<App>::start(crate::config()).unwrap();
    contime
        .apply([
            Added { time: -10, id: 3, value: 3 }.into(),
            Added { time: -10, id: 1, value: 1 }.into(),
            Added { time: 10, id: 5, value: 5 }.into(),
        ])
        .unwrap();
    contime.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    contime.apply([Cleared { time: -10, id: 99 }.into()]).unwrap();
    contime.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let states = contime.query_at(10, [7]).unwrap();
    let AppSnapshotLanes::State(state) = &*states[0];
    assert_eq!(state.values, [1, 3, 5], "clear runs before adds at -10, not after the later timestamp");
    assert_eq!(state.batches, [(-10, 0), (10, 3)]);
    assert!(contime.errors().is_empty());
    contime.shutdown();
}

#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added],snapshots=[State])]
struct Incomplete;
#[test]
#[should_panic(expected = "absent from the module")]
fn missing_registered_event_is_reported_before_manual_processing_starts() {
    let _ = ManualConTime::<Incomplete>::new(ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(16).unwrap(),
        checkpoints: CheckpointConfig { interval: 32 },
    });
}

#[derive(Clone, Default)]
struct Duplicate {
    time: i64,
    id: u128,
}
impl contime::checkpoints::Snapshot for Duplicate {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl SnapshotId for Duplicate {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}
impl SnapshotApply for Duplicate {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<Added>(|e, emit| emit(e.id)).event::<Added>(|_, emit| emit(99));
    }
    fn create(id: u128, _: EventRef<'_>) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        assert_eq!(batch.events::<Cleared>().count(), 0);
    }
}

#[test]
fn duplicate_registration_keeps_one_route_and_views_can_be_empty() {
    let event = Added { time: 0, id: 7, value: 1 };
    let set = contime::module::EventSet::<Duplicate>::new();
    let mut ids = Vec::new();
    set.snapshot_ids(EventRef::new(&event), &mut |id| ids.push(id));
    assert_eq!(ids, [7]);
    assert!(set.create(99, EventRef::new(&event)).is_none());
    let mut state = set.create(7, EventRef::new(&event)).unwrap();
    let mut events = [&event].into_iter();
    contime::module::apply_batch(
        &mut state,
        contime::checkpoints::ApplyBatch { snapshot_id: 7, time: 0, history_event_count: 0, events: &mut events },
    );
}

mod cached {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DEFINITIONS: AtomicUsize = AtomicUsize::new(0);
    #[derive(Clone, Default)]
    struct Cached {
        time: i64,
        id: u128,
    }
    impl contime::checkpoints::Snapshot for Cached {
        type Time = i64;
        fn time(&self) -> &Self::Time {
            &self.time
        }
        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    impl SnapshotId for Cached {
        fn snapshot_id(&self) -> u128 {
            self.id
        }
    }
    impl SnapshotApply for Cached {
        fn define(events: &mut EventSetBuilder<Self>) {
            DEFINITIONS.fetch_add(1, Ordering::SeqCst);
            events.event::<Added>(|e, emit| emit(e.id));
        }
        fn create(id: u128, _: EventRef<'_>) -> Self {
            Self { id, ..Self::default() }
        }
        fn apply_batch(&mut self, _: impl ApplyContext<Self>) {}
    }
    #[derive(ContimeModule)]
    #[contime_module(time=i64,events=[super::Added],snapshots=[Cached])]
    struct Child;
    #[derive(ContimeModule)]
    #[contime_module(time=i64,modules=[Child,Child])]
    struct Composed;
    #[test]
    fn definition_is_cached_across_repeated_construction_and_duplicate_composition() {
        let _child = Child;
        for _ in 0..3 {
            let contime = ConTime::<Composed>::start(crate::config()).unwrap();
            contime.shutdown();
            let _manual = ManualConTime::<Composed>::new(ManualConfig {
                queue_capacity: std::num::NonZeroUsize::new(16).unwrap(),
                checkpoints: CheckpointConfig { interval: 32 },
            });
        }
        assert_eq!(DEFINITIONS.load(Ordering::SeqCst), 1);
    }
}

#[derive(ContimeModule)]
#[contime_module(time=i64)]
struct Empty;
#[test]
fn empty_module_definitions_remain_valid() {
    Empty::validate();
    let contime = ConTime::<Empty>::start(crate::config()).unwrap();
    assert!(contime.query_at(0, [7]).unwrap().is_empty());
    contime.shutdown();
}

#[test]
fn type_phases_survive_historical_queries_pruning_and_boundary_late_input() {
    use std::{cell::RefCell, rc::Rc};
    let mut contime = ManualConTime::<App>::new(ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(16).unwrap(),
        checkpoints: CheckpointConfig { interval: 2 },
    });
    contime
        .apply([
            Added { time: -10, id: 3, value: 3 }.into(),
            Added { time: -10, id: 1, value: 1 }.into(),
            Cleared { time: -10, id: 99 }.into(),
            Added { time: 10, id: 5, value: 5 }.into(),
        ])
        .unwrap();
    contime.advance_to(20).unwrap();
    while contime.process().unwrap() {}
    let values = Rc::new(RefCell::new(Vec::new()));
    let output = values.clone();
    contime
        .query_at(-10, [7], move |states| {
            let AppSnapshotLanes::State(state) = &*states[0];
            *output.borrow_mut() = state.values.clone();
        })
        .unwrap();
    while contime.process().unwrap() {}
    assert_eq!(*values.borrow(), [1, 3]);
    contime.prune_before(0).unwrap();
    while contime.process().unwrap() {}
    contime.apply([Added { time: 0, id: 4, value: 4 }.into()]).unwrap();
    let output = values.clone();
    contime
        .query_at(20, [7], move |states| {
            let AppSnapshotLanes::State(state) = &*states[0];
            *output.borrow_mut() = state.values.clone();
        })
        .unwrap();
    while contime.process().unwrap() {}
    assert_eq!(*values.borrow(), [1, 3, 4, 5]);
    assert!(contime.take_rejections().is_empty());
}
