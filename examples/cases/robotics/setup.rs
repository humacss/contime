//! The independent robot snapshot uses the existing Manual/Runtime adapters.
use super::model::{Reading, Robot, SensorEvent};
use crate::definition::{Example, Time};
pub struct RoboticsExample;
impl Example for RoboticsExample {
    type Event = SensorEvent;
    type State = Robot;
    const TITLE: &'static str = "Robotics / delayed perception";
    const VALUE_LABEL: &'static str = "Observations";
    fn event(id: u128, time: Time, targets: Vec<u128>, _: i64) -> SensorEvent {
        SensorEvent { id, time, robot_id: targets.first().copied().unwrap_or(1), reading: Reading::Plan }
    }
    fn id(state: &Robot) -> u128 {
        state.robot_id
    }
    fn count(state: &Robot) -> u64 {
        state.events
    }
    fn value(state: &Robot) -> i64 {
        state.observations as i64
    }
    fn record(state: &mut Robot, event: &SensorEvent) {
        state.apply_reading(event);
    }
}
