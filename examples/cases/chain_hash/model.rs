//! A counter snapshot with an order-sensitive digest of every applied event.
//!
//! `CounterChanged -> Counter` uses the same routing/application contracts as the
//! simple counter case. The additional hash makes event order observable even
//! when the numeric total is unchanged.
use contime::prelude::*;

pub use crate::definition::Time;

/// Add an increment and extend the hash chain of every selected counter.
#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
pub struct CounterChanged {
    /// Stable source-event identity; also breaks equal-timestamp ordering ties.
    #[contime(id)]
    pub event_id: u128,
    /// Distinct snapshot IDs receiving this same event.
    pub counter_ids: Vec<u128>,
    /// Application time; arrival order may differ from timestamp order.
    #[contime(time)]
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

/// One snapshot's numeric total, application count and complete ordered digest.
#[derive(Debug, PartialEq, Eq, Clone, Default, ContimeSnapshot)]
#[contime(event = CounterChanged, id = counter_event_ids)]
pub struct Counter {
    /// Stable identity of the counter receiving events.
    #[contime(id)]
    pub counter_id: u128,
    /// State time, maintained by ConTime.
    #[contime(time)]
    pub time: Time,
    /// Sum of the applied increments; addition alone cannot prove event order.
    pub value: i64,
    /// Event applications represented in this snapshot state.
    pub event_count: u64,
    /// BLAKE3 chain digest; the initial state contains 32 zero bytes.
    pub hash: [u8; 32],
}

fn counter_event_ids(event: &CounterChanged, emit: &mut dyn FnMut(u128)) {
    for id in &event.counter_ids {
        emit(*id);
    }
}

// Fanout selects a destination from a collection, so initialization stays explicit.
impl contime::SnapshotCreate for Counter {
    fn create(snapshot_id: u128, _event: contime::EventRef<'_>) -> Self {
        Self { counter_id: snapshot_id, ..Self::default() }
    }
}
impl SnapshotApply for Counter {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for change in batch.events::<CounterChanged>() {
            self.apply_change(change);
        }
    }
}

impl Counter {
    /// Apply one chain link; ConTime and the reference use separate state instances.
    pub fn apply_change(&mut self, event: &CounterChanged) {
        self.value += event.delta;
        self.event_count += 1;
        self.hash = chain_hash(&self.hash, event);
    }
}

crate::definition::single_lane!(CounterChanged => Counter);

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
