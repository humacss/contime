//! Connect `CounterChanged -> Counter` to the shared diagnostic harness.
//!
//! `model.rs` declares routing and application through the derives; this adapter
//! selects those types and exposes their total, count and digest to validation
//! and presentation. It does not start ConTime or register a second lane.
use super::model::{Counter, CounterChanged};
use crate::definition::{Example, Time};
/// Explain the hash-chain model to shared example tools.
pub struct HashExample;
impl Example for HashExample {
    type Event = CounterChanged;
    type State = Counter;
    const TITLE: &'static str = "Chain hash verification";
    const VALUE_LABEL: &'static str = "Total";
    fn event(id: u128, time: Time, targets: Vec<u128>, value: i64) -> CounterChanged {
        CounterChanged { event_id: id, time, counter_ids: targets, delta: value }
    }
    fn id(state: &Counter) -> u128 {
        state.counter_id
    }
    fn count(state: &Counter) -> u64 {
        state.event_count
    }
    fn value(state: &Counter) -> i64 {
        state.value
    }
    fn record(state: &mut Counter, event: &CounterChanged) {
        state.apply_change(event);
    }
    fn hash(state: &Counter) -> Option<[u8; 32]> {
        Some(state.hash)
    }
}
