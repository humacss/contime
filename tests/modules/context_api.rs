use contime::prelude::*;
use std::{cell::Cell, num::NonZeroUsize, rc::Rc};

struct Changed {
    time: i64,
    id: u128,
    targets: Vec<u128>,
    value: char,
    creations: Rc<Cell<usize>>,
}
impl contime::checkpoints::Event for Changed {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl EventId for Changed {
    fn event_id(&self) -> u128 {
        self.id
    }
}

#[derive(Clone, Default)]

struct List {
    time: i64,
    id: u128,
    values: Vec<char>,
}
impl contime::checkpoints::Snapshot for List {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl SnapshotId for List {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}
impl SnapshotApply for List {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Changed>(|event, emit| {
            for &id in &event.targets {
                emit(id);
            }
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<Changed>() {
            let mut snapshot = {
                let initialize: fn(u128, &Changed) -> Self = |id, event| {
                    event.creations.set(event.creations.get() + 1);
                    Self { id, ..Self::default() }
                };
                initialize(snapshot_id, event)
            };
            for id in &(event.targets) {
                snapshot.id = *id;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        assert_eq!(batch.snapshot_id(), self.snapshot_id());
        let time = *batch.time();
        for event in batch.events::<Changed>() {
            assert_eq!(event.time, time);
            self.values.push(event.value);
        }
        assert!(batch.events::<Changed>().count() > 0);
    }
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Changed],snapshots=[List])]
struct App;
#[test]
fn context_routes_before_creation_and_replays_multiple_histories() {
    let creations = Rc::new(Cell::new(0));
    let event = |time, id, value| Changed { time, id, value, targets: vec![7, 9], creations: creations.clone() };
    let mut contime = ManualConTime::<App>::new(ManualConfig {
        queue_capacity: NonZeroUsize::new(16).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    let first = event(10, 1, 'c');
    let mut ids = Vec::new();
    contime::module::EventSet::<List>::new().snapshot_ids(contime::EventRef::new(&first), &mut |id| ids.push(id));
    assert_eq!(ids, [7, 9]);
    assert_eq!(creations.get(), 0, "routing must not initialize temporary snapshots");
    assert!(contime::module::EventSet::<List>::new().create(99, contime::EventRef::new(&first)).is_none());
    assert_eq!(creations.get(), 0, "unrelated destinations must not initialize snapshots");
    contime.apply([first.into()]).unwrap();
    while contime.process().unwrap() {}
    assert_eq!(creations.get(), 2, "one initializer per destination, not per candidate key");
    contime.apply([event(-10, 2, 'a').into(), event(0, 3, 'b').into()]).unwrap();
    let observed = Rc::new(std::cell::RefCell::new(Vec::new()));
    let output = observed.clone();
    contime
        .query_at(10, [7, 9], move |states| {
            *output.borrow_mut() = states
                .into_iter()
                .map(|state| {
                    let AppSnapshotLanes::List(list) = *state;
                    (list.snapshot_id(), list.values)
                })
                .collect();
        })
        .unwrap();
    while contime.process().unwrap() {}
    let mut actual = observed.borrow().clone();
    actual.sort_by_key(|row| row.0);
    assert_eq!(actual, [(7, vec!['a', 'b', 'c']), (9, vec!['a', 'b', 'c'])]);
    assert!(contime.take_rejections().is_empty());
}

#[derive(Clone, Default)]

struct StoredIdentity {
    time: i64,
    id: u128,
    count: usize,
}
impl contime::checkpoints::Snapshot for StoredIdentity {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl SnapshotId for StoredIdentity {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}

impl SnapshotApply for StoredIdentity {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Changed>(|event, emit| {
            for &id in &event.targets {
                emit(id + 100);
            }
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<Changed>() {
            let snapshot = {
                let initialize: fn(u128, &Changed) -> Self = |id, _event| Self { id, ..Self::default() };
                initialize(snapshot_id, event)
            };
            {
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        self.count += batch.events::<Changed>().count();
    }
}
#[test]
fn custom_creation_can_store_selected_ids_without_key_mapping() {
    let event = Changed { time: 0, id: 1, targets: vec![7, 9], value: 'x', creations: Rc::new(Cell::new(0)) };
    let snapshot = contime::module::EventSet::<StoredIdentity>::new().create(109, contime::EventRef::new(&event)).unwrap();
    assert_eq!(snapshot.snapshot_id(), 109);
    assert!(contime::module::EventSet::<StoredIdentity>::new().create(7, contime::EventRef::new(&event)).is_none());
}
