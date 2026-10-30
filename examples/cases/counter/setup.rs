//! Connect the counter model to the shared simulator/monitor example harness.
//!
//! The lane's application contract is already in `model.rs`:
//! `CounterChanged` names counter IDs, and the snapshot derive supplies the application rule. The environment instantiates Runtime or Manual
//! with these event/state types. There is no additional lane registry here.
//!
//! `Example` supplies construction, inspection and canonical-reference hooks for
//! the diagnostic harness; consumers using ConTime do not implement this trait.
use super::model::{Counter, CounterChanged};
use crate::definition::{Example, Time};
/// Select the event and snapshot types and explain them to shared example tools.
pub struct CounterExample;
impl Example for CounterExample {
    type Event = CounterChanged;
    type State = Counter;
    const TITLE: &'static str = "Counter";
    const VALUE_LABEL: &'static str = "Total";
    fn event(id: u128, time: Time, targets: Vec<u128>, value: i64) -> CounterChanged {
        CounterChanged { id, time, targets, value }
    }
    fn id(state: &Counter) -> u128 {
        state.id
    }
    fn count(state: &Counter) -> u64 {
        state.events
    }
    fn value(state: &Counter) -> i64 {
        state.value
    }
    // Build reference state separately from ConTime's stored snapshots.
    fn record(state: &mut Counter, event: &CounterChanged) {
        state.value += event.value;
        state.events += 1;
        state.time = event.time;
    }
}
