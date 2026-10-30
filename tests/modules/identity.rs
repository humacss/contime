use contime::ContimeEvent;

// This payload deliberately implements no Hash trait.
struct Payload(i64);

#[derive(ContimeEvent)]
#[contime_event(id = |event| event.id)]
struct Explicit {
    time: i64,
    id: u128,
    payload: Payload,
}

#[derive(ContimeEvent)]
#[contime_event(id = |event| event.calculate_id())]
struct Computed {
    time: i64,
    sequence: u64,
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

#[derive(contime::ContimeEvent)]
#[contime_event(id = |event: &Self| event.id)]
struct KeyEvent {
    time: i64,
    id: u128,
    key: Key,
}

#[derive(contime::ContimeSnapshot)]
#[contime_snapshot(id = |key: &Key| key.0, events = [KeyEvent], apply = {
    self.count += batch.events.count();
})]
struct KeyState {
    time: i64,
    key: Key,
    count: usize,
}

#[test]
fn snapshot_callbacks_use_the_same_unhashed_id_for_queries_routing_and_creation() {
    use contime::module::Route;
    for id in [0, 42, u128::MAX] {
        let event = KeyEvent { time: -1, id: 7, key: Key(id) };
        assert_eq!(event.event_id(), 7);
        assert_eq!(KeyState::snapshot_id(Key(id)), id);
        let mut routed = Vec::new();
        KeyState::snapshot_ids(&event, &mut |id| routed.push(id));
        assert_eq!(routed, [id]);
        assert_eq!(KeyState::create(id, &event).unwrap().key.0, id);
        assert!(KeyState::create(id.wrapping_add(1), &event).is_none());
    }
}

#[derive(contime::ContimeEvent)]
#[contime_event(id = |event| event.id)]
struct PositionedInput {
    id: u128,
    time: i64,
    key: u128,
    origin: i64,
    delta: i64,
}

#[derive(contime::ContimeSnapshot)]
#[contime_snapshot(
    id = |key| *key,
    events = [PositionedInput],
    create = |_id, event| Self {position:event.origin,..Self::default()},
    apply = {
        for event in batch.events {
            let PositionedStateEvent::PositionedInput(input) = event;
            self.position += input.delta;
            self.events += 1;
        }
    }
)]
struct PositionedState {
    time: i64,
    key: u128,
    position: i64,
    events: usize,
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
            .query_at(time, [PositionedState::snapshot_id(42)], move |states| {
                let PositionsSnapshotLanes::PositionedState(state) = &*states[0];
                assert_eq!(state.key, 42);
                output.borrow_mut().push((state.position, state.events));
            })
            .unwrap();
    }
    while contime.process().unwrap() {}
    assert_eq!(*observed.borrow(), [(100, 0), (107, 1)]);
}
