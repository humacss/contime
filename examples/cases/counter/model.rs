//! One event type adds to one snapshot type: `CounterChanged -> Counter`.
//!
//! `Event` supplies application time. `Input` supplies a stable event identity
//! and the counter IDs to update. `ApplyEvents` defines the state change for each
//! selected counter. None of these contracts starts or selects an execution mode.
pub use crate::definition::Time;
use contime::checkpoints::{ApplyBatch, ApplyEvents, Event, Snapshot};
use contime::Input;

/// Add `value` to each counter listed in `targets` at `time`.
///
/// Live simulation uses an increment of one. The model also supports other
/// signed increments; this field is a change, not a replacement counter value.
#[derive(Clone)]
pub struct CounterChanged {
    /// Stable source-event identity; retained retries reuse this ID.
    pub id: u128,
    /// Application time, using the shared example time type.
    pub time: Time,
    /// Distinct counter snapshot IDs affected by this event.
    pub targets: Vec<u128>,
    /// Signed amount added to each selected counter.
    pub value: i64,
}
impl Event for CounterChanged {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
// Routing selects counters independently; a shared event is not a transaction.
impl Input for CounterChanged {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for id in &self.targets {
            emit(*id);
        }
    }
}
/// State of one counter, reconstructed at the time requested by a query.
#[derive(Clone, Default)]
pub struct Counter {
    /// Counter identity, copied from a destination emitted by `snapshot_ids`.
    pub id: u128,
    /// State time, maintained by ConTime through `Snapshot::set_time`.
    pub time: Time,
    /// Sum of the increments applied to this snapshot.
    pub value: i64,
    /// Event applications represented in this state, used by reference validation.
    pub events: u64,
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
// This implementation connects the event type to its snapshot type.
impl ApplyEvents<CounterChanged> for Counter {
    fn create(id: u128, _: &CounterChanged) -> Self {
        // Initialize the counter; the first event is applied through the batch.
        Self { id, ..Self::default() }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, CounterChanged>) {
        for change in batch.events {
            self.value += change.value;
            self.events += 1;
        }
    }
}
