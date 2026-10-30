//! Static application definitions shared by managed and manual execution.
use crate::contime_core::{checkpoints, Input};

/// Metadata supplied by an application event, independent of its registration.
pub trait Event: checkpoints::Event {
    /// Stable identity used for ordering and duplicate suppression.
    fn event_id(&self) -> u128;
}
/// A borrowed concrete event view into an aggregate lane.
pub trait View<E> {
    /// Borrows the event when this lane contains the requested type.
    fn get(&self) -> Option<&E>;
}
/// Routing and identity initialization for one event/snapshot relationship.
pub trait Route<E>: checkpoints::Snapshot {
    /// Visits all histories of this snapshot type targeted by the event.
    fn snapshot_ids(event: &E, emit: &mut impl FnMut(u128));
    /// Initializes the requested identity, or returns `None` for another route.
    fn create(snapshot_id: u128, event: &E) -> Option<Self>
    where
        Self: Sized;
}
/// Applies a concrete snapshot's projected events from an aggregate lane.
pub trait Apply<I>: checkpoints::Snapshot {
    /// Applies one complete timestamp batch without copying event payloads.
    fn apply(&mut self, batch: checkpoints::ApplyBatch<'_, '_, Self::Time, I>);
}
/// Borrowed snapshot-specific events and their canonical batch metadata.
pub struct ApplyBatch<T, B> {
    /// Destination history identity.
    pub snapshot_id: u128,
    /// Timestamp shared by this batch.
    pub time: T,
    /// Number of canonical history events preceding this batch.
    pub history_event_count: u64,
    /// Snapshot-specific borrowed event views, in canonical order.
    pub events: B,
}
/// A complete statically dispatched application definition.
///
/// All nested modules, inputs and snapshots share exactly `Time`. A module is
/// a definition, not a running process; composing it starts no threads.
pub trait Module: 'static {
    /// Ordered application time domain, including its explicit minimum.
    type Time: checkpoints::Timestamp + crate::contime_worker::AdvanceTime + 'static;
    /// Registered inputs and their event-to-snapshot routing.
    type EventLanes: Input<Time = Self::Time>;
    /// Registered snapshot states and deterministic application.
    type SnapshotLanes: checkpoints::Snapshot<Time = Self::Time> + checkpoints::ApplyEvents<Self::EventLanes> + 'static;
}
/// Routes a heterogeneous input through one snapshot's registered event views.
pub trait Routes<I>: checkpoints::Snapshot {
    /// Visits this snapshot type's destinations for a borrowed aggregate input.
    fn snapshot_ids(event: &I, emit: &mut impl FnMut(u128));
    /// Initializes one selected snapshot, if this aggregate input targets it.
    fn create(snapshot_id: u128, event: &I) -> Option<Self>
    where
        Self: Sized;
}
