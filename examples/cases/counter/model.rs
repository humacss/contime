//! One event type adds to one snapshot type: `CounterChanged -> Counter`.
//!
//! The derives declare the supplied event ID, destination keys and application
//! rule. The shared single-lane adapter connects them to the live host's concrete
//! types. These definitions do not start or select an execution mode.
pub use crate::definition::Time;
use contime::{ContimeEvent, ContimeSnapshot};

/// Add `value` to each counter listed in `targets` at `time`.
///
/// Live simulation uses an increment of one. The model also supports other
/// signed increments; this field is a change, not a replacement counter value.
#[derive(Clone, ContimeEvent)]
#[contime_event(id = |event| event.id)]
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
/// State of one counter, reconstructed at the time requested by a query.
#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |id| *id, events = [CounterChanged => each(self.targets, |id| [*id])], apply = {
    for event in batch.events {
        let CounterEvent::CounterChanged(change) = event;
        self.value += change.value;
        self.events += 1;
    }
})]
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

crate::definition::single_lane!(CounterChanged => Counter);
