use contime_core::{
    checkpoints::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, CheckpointConfig, Event, EventBatch, Snapshot, Timestamp},
    CheckpointStorage, CheckpointStorageConfig, Input, SharedEvent,
};
use contime_worker::SnapshotStore;
use std::{cell::Cell, rc::Rc};
#[derive(Clone, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, rhs: &Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}
struct LocalInput {
    id: u128,
    time: Time,
    value: Rc<u64>,
}
impl Event for LocalInput {
    type Time = Time;
    fn time(&self) -> Time {
        self.time.clone()
    }
}
impl Input for LocalInput {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(7);
    }
}
#[derive(Clone, Default)]
struct State {
    time: Time,
    values: Vec<u64>,
    _local: Rc<()>,
}
impl Snapshot for State {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl ApplyEvents<LocalInput> for State {
    fn create(_: u128, _: &LocalInput) -> Self {
        Self::default()
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, LocalInput>) {
        self.values.extend(batch.events.map(|e| *e.value));
    }
}
struct Hooks(Rc<Cell<usize>>);
impl ApplyWrapper<State, LocalInput> for Hooks {
    fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, Time, LocalInput>, inner: &mut ApplyInner<'_, State>) {
        self.0.set(self.0.get() + 1);
        inner.apply_event_batch(batch);
    }
}
fn event(id: u128, time: u64, value: u64) -> SharedEvent<LocalInput> {
    LocalInput { id, time: Time(time), value: Rc::new(value) }.into()
}
#[test]
fn shared_storage_accepts_thread_local_state_and_preserves_history_semantics() {
    let mut store = CheckpointStorage::<LocalInput, State, Hooks>::create(
        7,
        &CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 1 } },
        &Time(0),
    );
    let effects = Rc::new(Cell::new(0));
    let mut hooks = Hooks(effects.clone());
    assert!(store.insert(event(1, 5, 50), &Time(0)).changed);
    assert!(store.insert(event(2, 10, 100), &Time(0)).changed);
    store.process_until(&Time(11), &mut hooks);
    assert!(effects.get() > 0);
    assert!(store.insert(event(3, 7, 70), &Time(0)).changed);
    assert!(!store.insert(event(1, 5, 50), &Time(0)).changed);
    let before = effects.get();
    assert_eq!(store.query(Time(11), &mut hooks).unwrap().values, [50, 70, 100]);
    assert_eq!(store.query(Time(6), &mut hooks).unwrap().values, [50]);
    assert_eq!(effects.get(), before, "queries must remain silent");
    store.process_until(&Time(11), &mut hooks);
    let before = effects.get();
    store.forward(&Time(8), &mut hooks);
    store.prune();
    assert_eq!(effects.get(), before, "forwarding must remain silent");
    assert!(store.query(Time(6), &mut hooks).is_none());
    assert_eq!(store.query(Time(11), &mut hooks).unwrap().values, [50, 70, 100]);
    assert_eq!(store.query_events(&Time(0), &Time(20)).iter().map(|e| e.event_id()).collect::<Vec<_>>(), [2]);
    assert!(!store.insert(event(4, 7, 99), &Time(8)).rejections.is_empty());
}
