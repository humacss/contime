//! ID helpers delegate to user functions without choosing an identity scheme.
use contime::checkpoints::{Event, Snapshot};
use contime::prelude::*;
#[derive(ContimeEvent)]
#[contime(id = change_id)]
struct Change {
    #[contime(time)]
    at: i64,
    owner: u128,
    value: char,
}
fn change_id(value: &Change) -> u128 {
    contentid::typed_content_id_u128(std::any::type_name_of_val(value), &(value.at, value.owner, u32::from(value.value)))
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id=state_id)]
struct State {
    #[contime(time)]
    at: i64,
    owner: u128,
    values: Vec<char>,
}
fn state_id(value: &State) -> u128 {
    state_key(value.owner)
}
fn state_key(owner: u128) -> u128 {
    contentid::typed_content_id_u128(std::any::type_name::<State>(), &owner)
}
impl contime::SnapshotEvents for State {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<Change>(|e, emit| emit(state_key(e.owner)));
    }
}
impl contime::SnapshotCreate for State {
    fn create(_: u128, event: EventRef<'_>) -> Self {
        Self { owner: event.get::<Change>().unwrap().owner, ..Self::default() }
    }
}
impl SnapshotApply for State {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for e in batch.events::<Change>() {
            self.values.push(e.value);
        }
    }
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Change],snapshots=[State])]
struct App;
#[test]
fn content_id_callbacks_replay_and_preserve_snapshot_identity() {
    let c = ConTime::<App>::start(crate::config()).unwrap();
    let change = |at, value| Change { at, owner: 7, value };
    assert_eq!(change(-10, 'a').event_id(), change(-10, 'a').event_id());
    assert_ne!(change(-10, 'a').event_id(), change(-10, 'b').event_id());
    c.apply([change(10, 'c').into(), change(-10, 'a').into(), change(0, 'b').into(), change(-10, 'a').into()]).unwrap();
    c.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let states = c.query_at(10, [state_key(7)]).unwrap();
    let AppSnapshotLanes::State(s) = &*states[0];
    assert_eq!(s.values, ['a', 'b', 'c']);
    assert_eq!(s.snapshot_id(), state_key(7));
    assert!(c.errors().is_empty());
    c.shutdown();
}
#[derive(ContimeEvent)]
#[contime(id=|event|event.id)]
struct ManualTime {
    id: u128,
    at: i64,
}
impl Event for ManualTime {
    type Time = i64;
    fn time(&self) -> i64 {
        self.at
    }
}
#[derive(Clone, ContimeSnapshot)]
#[contime(id=manual_state_id)]
struct ManualState {
    owner: u128,
    at: i64,
}
fn manual_state_id(s: &ManualState) -> u128 {
    s.owner
}
impl Snapshot for ManualState {
    type Time = i64;
    fn time(&self) -> &i64 {
        &self.at
    }
    fn set_time(&mut self, t: i64) {
        self.at = t;
    }
}
#[test]
fn id_only_helpers_preserve_manual_timestamps() {
    let e = ManualTime { id: 99, at: -5 };
    assert_eq!(e.event_id(), 99);
    assert_eq!(e.time(), -5);
    let mut s = ManualState { owner: 7, at: -5 };
    s.set_time(10);
    assert_eq!(s.snapshot_id(), 7);
    assert_eq!(s.time(), &10);
}
#[derive(ContimeEvent)]
#[contime(id=generic_id::<T>)]
struct Generic<T> {
    #[contime(time)]
    at: i64,
    id: u128,
    payload: T,
}
fn generic_id<T>(e: &Generic<T>) -> u128 {
    e.id
}
#[test]
fn generic_callbacks_add_no_payload_bounds() {
    struct Local(std::rc::Rc<()>);
    let e = Generic { at: -1, id: 7, payload: Local(std::rc::Rc::new(())) };
    assert_eq!(e.event_id(), 7);
    assert_eq!(std::rc::Rc::strong_count(&e.payload.0), 1);
}
