//! Thread safety belongs to the execution boundary, not module definitions.
use contime::{
    checkpoints::{Event, Snapshot},
    ApplyContext, ContimeModule, EventId, EventRef, EventSetBuilder, SnapshotApply, SnapshotId,
};
use std::{cell::RefCell, num::NonZeroUsize, rc::Rc};
struct LocalInput {
    at: i64,
    id: u128,
    text: Rc<String>,
}
impl Event for LocalInput {
    type Time = i64;
    fn time(&self) -> i64 {
        self.at
    }
}
impl EventId for LocalInput {
    fn event_id(&self) -> u128 {
        self.id
    }
}
#[derive(Clone, Default)]
struct LocalState {
    at: i64,
    values: Rc<Vec<String>>,
}
impl Snapshot for LocalState {
    type Time = i64;
    fn time(&self) -> &i64 {
        &self.at
    }
    fn set_time(&mut self, t: i64) {
        self.at = t;
    }
}
impl SnapshotId for LocalState {
    fn snapshot_id(&self) -> u128 {
        7
    }
}
impl SnapshotApply for LocalState {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<LocalInput>(|_, emit| emit(7));
    }
    fn create(_: u128, _: EventRef<'_>) -> Self {
        Self::default()
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for e in batch.events::<LocalInput>() {
            Rc::make_mut(&mut self.values).push((*e.text).clone());
        }
    }
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[LocalInput],snapshots=[LocalState])]
struct Local;
#[test]
fn manual_accepts_non_send_events_and_snapshots() {
    let mut contime = contime::manual::ManualConTime::<Local>::new(contime::manual::ManualConfig {
        queue_capacity: NonZeroUsize::new(16).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    contime.apply([LocalInput { at: 10, id: 2, text: Rc::new("b".into()) }.into()]).unwrap();
    while contime.process().unwrap() {}
    contime.apply([LocalInput { at: -10, id: 1, text: Rc::new("a".into()) }.into()]).unwrap();
    let values = Rc::new(RefCell::new(Vec::new()));
    let output = values.clone();
    contime
        .query_at(10, [7], move |states| {
            let LocalSnapshotLanes::LocalState(s) = &*states[0];
            output.borrow_mut().extend(s.values.iter().cloned());
        })
        .unwrap();
    while contime.process().unwrap() {}
    assert_eq!(*values.borrow(), ["a", "b"]);
    assert!(contime.take_rejections().is_empty());
}
