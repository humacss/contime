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
impl contime::checkpoints::Event for CounterChanged {
    type Time = Time;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl EventId for CounterChanged {
    fn event_id(&self) -> u128 {
        self.id
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
impl contime::checkpoints::Snapshot for Counter {
    type Time = Time;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl SnapshotId for Counter {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}
impl SnapshotApply for Counter {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<CounterChanged>(|event, emit| {
            for id in &event.targets {
                emit(*id);
            }
        });
    }
    fn create(snapshot_id: u128, _event: contime::EventRef<'_>) -> Self {
        Self { id: snapshot_id, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for change in batch.events::<CounterChanged>() {
            self.value += change.value;
            self.events += 1;
        }
    }
}

crate::definition::single_lane!(CounterChanged => Counter);
