//! Select the movement lane for the existing Runtime/Manual execution adapters.
//! `Example` is an example-support contract; consumers only need Input/Snapshot/ApplyEvents.
use super::model::{Character, Direction, Motion, MovementInput, Time, SECOND};
use crate::definition::Example;
pub struct MovementExample;
impl Example for MovementExample {
    type Event = MovementInput;
    type State = Character;
    const TITLE: &'static str = "Client movement";
    const VALUE_LABEL: &'static str = "X";
    fn event(id: u128, time: Time, targets: Vec<u128>, value: i64) -> MovementInput {
        MovementInput {
            id,
            client_id: targets.first().copied().unwrap_or(1),
            time,
            direction: Direction::from_index(value.unsigned_abs()),
            motion: Motion::default(),
        }
    }
    fn id(state: &Character) -> u128 {
        state.client_id
    }
    fn count(state: &Character) -> u64 {
        state.events
    }
    fn value(state: &Character) -> i64 {
        (state.position[0] / SECOND) as i64
    }
    fn record(state: &mut Character, event: &MovementInput) {
        state.apply_input(event);
    }
}
