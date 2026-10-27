//! A counter snapshot with an order-sensitive digest of every applied event.
//!
//! `CounterChanged -> Counter` uses the same routing/application contracts as the
//! simple counter case. The additional hash makes event order observable even
//! when the numeric total is unchanged.
use contime::checkpoints::{ApplyBatch, ApplyEvents, Event, Snapshot};
use contime::Input;

pub use crate::definition::Time;

/// Add an increment and extend the hash chain of every selected counter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CounterChanged {
    /// Stable source-event identity; also breaks equal-timestamp ordering ties.
    pub event_id: u128,
    /// Distinct snapshot IDs receiving this same event.
    pub counter_ids: Vec<u128>,
    /// Application time; arrival order may differ from timestamp order.
    pub time: Time,
    /// Signed increment included in both the total and the hashed contents.
    pub delta: i64,
}

/// Extend the full previous digest with the complete event contents.
///
/// Fixed-width fields and an explicit target count make the encoding unambiguous.
/// The previous digest carries the retained prefix into each new chain link.
pub fn chain_hash(previous: &[u8; 32], event: &CounterChanged) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"contime-counter-chain-v1");
    hasher.update(previous);
    hasher.update(&event.event_id.to_le_bytes());
    hasher.update(&event.time.0.to_le_bytes());
    hasher.update(&event.delta.to_le_bytes());
    hasher.update(&(event.counter_ids.len() as u64).to_le_bytes());
    for id in &event.counter_ids {
        hasher.update(&id.to_le_bytes());
    }
    *hasher.finalize().as_bytes()
}

impl Event for CounterChanged {
    type Time = Time;

    fn time(&self) -> Time {
        self.time
    }
}

impl Input for CounterChanged {
    fn event_id(&self) -> u128 {
        self.event_id
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for &counter_id in &self.counter_ids {
            emit(counter_id);
        }
    }
}

/// One snapshot's numeric total, application count and complete ordered digest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counter {
    /// Stable identity of the counter receiving events.
    pub counter_id: u128,
    /// State time, maintained by ConTime.
    pub time: Time,
    /// Sum of the applied increments; addition alone cannot prove event order.
    pub value: i64,
    /// Event applications represented in this snapshot state.
    pub event_count: u64,
    /// BLAKE3 chain digest; the initial state contains 32 zero bytes.
    pub hash: [u8; 32],
}

impl Counter {
    /// Apply one chain link; ConTime and the reference use separate state instances.
    pub fn apply_change(&mut self, event: &CounterChanged) {
        self.value += event.delta;
        self.event_count += 1;
        self.hash = chain_hash(&self.hash, event);
    }
}

impl Snapshot for Counter {
    type Time = Time;

    fn time(&self) -> &Time {
        &self.time
    }

    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}

impl ApplyEvents<CounterChanged> for Counter {
    fn create(snapshot_id: u128, _first_event: &CounterChanged) -> Self {
        // Initialize the prefix; the first event is applied through the batch.
        Self { counter_id: snapshot_id, ..Self::default() }
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, CounterChanged>) {
        for event in batch.events {
            self.apply_change(event);
        }
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
