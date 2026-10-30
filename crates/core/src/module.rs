//! Static application definitions shared by managed and manual execution.
use crate::contime_core::{checkpoints, Input};

/// Metadata supplied by an application event, independent of its registration.
pub trait EventId: checkpoints::Event {
    /// Stable identity used for ordering and duplicate suppression.
    fn event_id(&self) -> u128;
}
/// Event identity contract used by aggregate lane adapters.
pub use EventId as Event;

/// Stable identity of an initialized snapshot, matching its history's routing ID.
pub trait SnapshotId {
    /// Reads this snapshot's identity without choosing an implicit ID policy.
    fn snapshot_id(&self) -> u128;
}

mod context;
mod event_set;
pub use context::{apply_batch, ApplyContext, SnapshotApply};
pub use event_set::{EventRef, EventSet, EventSetBuilder, EventSource};

/// A complete statically dispatched application definition.
///
/// All nested modules, inputs and snapshots share exactly `Time`. A module is
/// a definition, not a running process; composing it starts no threads.
pub trait Module: 'static {
    /// Validates event registrations before execution starts.
    fn validate() {}
    /// Ordered application time domain, including its explicit minimum.
    type Time: checkpoints::Timestamp + crate::contime_worker::AdvanceTime + 'static;
    /// Registered inputs and their event-to-snapshot routing.
    type EventLanes: Input<Time = Self::Time>;
    /// Registered snapshot states and deterministic application.
    type SnapshotLanes: checkpoints::Snapshot<Time = Self::Time> + checkpoints::ApplyEvents<Self::EventLanes> + 'static;
}
