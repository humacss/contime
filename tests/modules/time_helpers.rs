//! Timestamp helpers are optional adapters over the existing ordinary traits.
use contime::checkpoints::{Event, Snapshot};
use contime::prelude::*;
use std::time::Duration;

#[derive(ContimeEvent)]
struct Added {
    id: u128,
    owner: u128,
    #[contime(time)]
    occurred_at: i64,
    value: char,
}
impl EventId for Added {
    fn event_id(&self) -> u128 {
        self.id
    }
}
#[derive(Clone, Default, ContimeSnapshot)]
struct List {
    owner: u128,
    values: Vec<char>,
    #[contime(time)]
    processed_through: i64,
}
impl SnapshotId for List {
    fn snapshot_id(&self) -> u128 {
        self.owner
    }
}
impl contime::SnapshotEvents for List {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<Added>(|e, emit| emit(e.owner));
    }
}
impl contime::SnapshotCreate for List {
    fn create(owner: u128, _: EventRef<'_>) -> Self {
        Self { owner, ..Self::default() }
    }
}
impl SnapshotApply for List {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for e in batch.events::<Added>() {
            self.values.push(e.value);
        }
    }
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added],snapshots=[List])]
struct App;
#[test]
fn marked_fields_replay_late_events_across_negative_time() {
    let c = ConTime::<App>::start(crate::config()).unwrap();
    for (id, time, value) in [(3, 10, 'c'), (1, -10, 'a'), (2, 0, 'b')] {
        c.apply([Added { id, owner: 7, occurred_at: time, value }.into()]).unwrap();
        c.wait_until_idle(Duration::from_secs(3)).unwrap();
    }
    let states = c.query_at(10, [7]).unwrap();
    let AppSnapshotLanes::List(s) = &*states[0];
    assert_eq!(s.values, ['a', 'b', 'c']);
    assert_eq!(*s.time(), 10);
    assert!(c.errors().is_empty());
    c.shutdown();
}
#[derive(Clone, Default, Eq, PartialEq, Ord, PartialOrd, Debug)]
struct NonCopyTime(i64);
struct NoClone;
#[derive(ContimeEvent)]
struct Sample<'a, T, const N: usize> {
    payload: &'a [T; N],
    #[contime(time)]
    observed: NonCopyTime,
}
#[derive(ContimeEvent)]
struct WithWhere<T, U>
where
    T: Clone + PartialEq,
{
    #[contime(time)]
    when: T,
    payload: U,
}
#[derive(ContimeSnapshot)]
struct Local<T> {
    payload: std::rc::Rc<T>,
    #[contime(time)]
    observed: NonCopyTime,
}
impl<T> Clone for Local<T> {
    fn clone(&self) -> Self {
        Self { payload: self.payload.clone(), observed: self.observed.clone() }
    }
}
#[test]
fn helpers_preserve_generics_and_require_no_payload_clone_or_thread_bounds() {
    let constrained = WithWhere { when: NonCopyTime(-20), payload: NoClone };
    assert_eq!(constrained.time(), NonCopyTime(-20));
    let _ = &constrained.payload;
    let payload = [NoClone];
    let sample = Sample { payload: &payload, observed: NonCopyTime(-10) };
    assert_eq!(sample.time(), NonCopyTime(-10));
    assert_eq!(sample.time(), NonCopyTime(-10));
    assert_eq!(sample.payload.len(), 1);
    let mut state = Local { payload: std::rc::Rc::new(NoClone), observed: NonCopyTime(-10) };
    state.set_time(NonCopyTime(5));
    assert_eq!(state.time(), &NonCopyTime(5));
    let _ = state.clone();
}
#[derive(ContimeEvent)]
struct TupleEvent(String, #[contime(time)] i64);
#[derive(Clone, ContimeSnapshot)]
struct TupleSnapshot(#[contime(time)] i64);
#[test]
fn tuple_fields_are_supported() {
    let e = TupleEvent("payload".into(), -5);
    assert_eq!(e.time(), -5);
    assert_eq!(e.0, "payload");
    let mut s = TupleSnapshot(-5);
    s.set_time(7);
    assert_eq!(s.time(), &7);
}

#[derive(serde::Serialize, ContimeEvent)]
struct SerializedEvent {
    #[serde(rename = "t")]
    #[contime(time)]
    occurred_at: i64,
}
#[test]
fn namespaced_field_markers_coexist_with_serde() {
    let event = SerializedEvent { occurred_at: -10 };
    assert_eq!(event.time(), -10);
    assert_eq!(serde_json::to_value(event).unwrap(), serde_json::json!({"t":-10}));
}
