//! One event type adds to one snapshot type: `CounterChanged -> Counter`.
//!
//! The event supplies IDs and destinations; traits define snapshot identity and
//! application. The shared single-lane adapter connects these definitions to the
//! live host's concrete types. These definitions do not start or select an execution mode.
pub use crate::definition::Time;
use contime::prelude::*;

/// Add `value` to each counter listed in `targets` at `time`.
///
/// Live simulation uses an increment of one. The model also supports other
/// signed increments; this field is a change, not a replacement counter value.
#[derive(Clone, ContimeEvent)]
pub struct CounterChanged {
    /// Stable source-event identity; retained retries reuse this ID.
    #[contime(id)]
    pub id: u128,
    /// Application time, using the shared example time type.
    #[contime(time)]
    pub time: Time,
    /// Distinct counter snapshot IDs affected by this event.
    pub targets: Vec<u128>,
    /// Signed amount added to each selected counter.
    pub value: i64,
}

/// State of one counter, reconstructed at the time requested by a query.
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(event = CounterChanged, id = counter_event_ids)]
pub struct Counter {
    /// Counter identity, copied from a destination emitted by `snapshot_ids`.
    #[contime(id)]
    pub id: u128,
    /// State time, maintained by ConTime through `Snapshot::set_time`.
    #[contime(time)]
    pub time: Time,
    /// Sum of the increments applied to this snapshot.
    pub value: i64,
    /// Event applications represented in this state, used by reference validation.
    pub events: u64,
}

fn counter_event_ids(event: &CounterChanged, emit: &mut dyn FnMut(u128)) {
    for id in &event.targets {
        emit(*id);
    }
}

// Fanout selects a destination from a collection, so initialization stays explicit.
impl contime::SnapshotCreate for Counter {
    fn create(snapshot_id: u128, _event: contime::EventRef<'_>) -> Self {
        Self { id: snapshot_id, ..Self::default() }
    }
}
impl SnapshotApply for Counter {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for change in batch.events::<CounterChanged>() {
            self.value += change.value;
            self.events += 1;
        }
    }
}

crate::definition::single_lane!(CounterChanged => Counter);
