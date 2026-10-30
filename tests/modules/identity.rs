use contime::EventId;
// This payload deliberately implements no Hash trait.
struct Payload(i64);

struct Explicit {
    time: i64,
    id: u128,
    payload: Payload,
}
impl contime::checkpoints::Event for Explicit {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl contime::EventId for Explicit {
    fn event_id(&self) -> u128 {
        self.id
    }
}

struct Computed {
    time: i64,
    sequence: u64,
}
impl contime::checkpoints::Event for Computed {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl contime::EventId for Computed {
    fn event_id(&self) -> u128 {
        self.calculate_id()
    }
}

impl Computed {
    fn calculate_id(&self) -> u128 {
        u128::from(self.sequence) + 100
    }
}

#[test]
fn supplied_and_computed_ids_are_returned_verbatim_without_hashing() {
    for id in [0, 42, u128::MAX] {
        let event = Explicit { time: -10, id, payload: Payload(3) };
        assert_eq!(event.event_id(), id);
        assert_eq!(contime::module::Event::event_id(&event), id);
        assert_eq!(event.payload.0, 3);
    }
    let event = Computed { time: 0, sequence: 7 };
    assert_eq!(event.event_id(), 107);
}

#[derive(Clone, Default)]
struct Key(u128);

struct KeyEvent {
    time: i64,
    id: u128,
    key: Key,
}
impl contime::checkpoints::Event for KeyEvent {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl contime::EventId for KeyEvent {
    fn event_id(&self) -> u128 {
        let event: &Self = self;
        event.id
    }
}

#[derive(Clone, Default)]

struct KeyState {
    time: i64,
    key: Key,
    count: usize,
}
impl contime::checkpoints::Snapshot for KeyState {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl KeyState {
    fn id_for(key: &Key) -> u128 {
        key.0
    }
}
impl contime::SnapshotId for KeyState {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.key)
    }
}
impl contime::SnapshotApply for KeyState {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<KeyEvent>(|event, emit| {
            emit(KeyState::id_for(&event.key));
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<KeyEvent>() {
            let mut snapshot = Self::default();
            {
                snapshot.key = event.key.clone();
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        self.count += batch.events::<KeyEvent>().count();
    }
}

#[test]
fn snapshot_traits_use_the_same_unhashed_id_for_queries_routing_and_creation() {
    for id in [0, 42, u128::MAX] {
        let event = KeyEvent { time: -1, id: 7, key: Key(id) };
        assert_eq!(event.event_id(), 7);
        assert_eq!(KeyState::id_for(&Key(id)), id);
        let mut routed = Vec::new();
        contime::module::EventSet::<KeyState>::new().snapshot_ids(contime::EventRef::new(&event), &mut |id| routed.push(id));
        assert_eq!(routed, [id]);
        assert_eq!(contime::module::EventSet::<KeyState>::new().create(id, contime::EventRef::new(&event)).unwrap().key.0, id);
        assert!(contime::module::EventSet::<KeyState>::new().create(id.wrapping_add(1), contime::EventRef::new(&event)).is_none());
    }
}

struct PositionedInput {
    id: u128,
    time: i64,
    key: u128,
    origin: i64,
    delta: i64,
}
impl contime::checkpoints::Event for PositionedInput {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl contime::EventId for PositionedInput {
    fn event_id(&self) -> u128 {
        self.id
    }
}

#[derive(Clone, Default)]

struct PositionedState {
    time: i64,
    key: u128,
    position: i64,
    events: usize,
}
impl contime::checkpoints::Snapshot for PositionedState {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl PositionedState {
    fn id_for(key: &u128) -> u128 {
        *key
    }
}
impl contime::SnapshotId for PositionedState {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.key)
    }
}
impl contime::SnapshotApply for PositionedState {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<PositionedInput>(|event, emit| {
            emit(PositionedState::id_for(&event.key));
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<PositionedInput>() {
            let mut snapshot = {
                let initialize: fn(u128, &PositionedInput) -> Self = |_id, event| Self { position: event.origin, ..Self::default() };
                initialize(snapshot_id, event)
            };
            {
                snapshot.key = event.key;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        for input in batch.events::<PositionedInput>() {
            self.position += input.delta;
            self.events += 1;
        }
    }
}

#[derive(contime::ContimeModule)]
#[contime_module(time=i64,events=[PositionedInput],snapshots=[PositionedState])]
struct Positions;

#[test]
fn custom_creation_preserves_initial_state_before_the_first_input_timestamp() {
    use std::{cell::RefCell, rc::Rc};
    let mut contime = contime::manual::ManualConTime::<Positions>::new(contime::manual::ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(16).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    contime.apply([PositionedInput { id: 1, time: 10, key: 42, origin: 100, delta: 7 }.into()]).unwrap();
    let observed = Rc::new(RefCell::new(Vec::new()));
    for time in [0, 10] {
        let output = observed.clone();
        contime
            .query_at(time, [PositionedState::id_for(&(42))], move |states| {
                let PositionsSnapshotLanes::PositionedState(state) = &*states[0];
                assert_eq!(state.key, 42);
                output.borrow_mut().push((state.position, state.events));
            })
            .unwrap();
    }
    while contime.process().unwrap() {}
    assert_eq!(*observed.borrow(), [(100, 0), (107, 1)]);
}
