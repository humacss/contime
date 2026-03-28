use std::fmt::Debug;

/// A point-in-time state for one logical entity in `contime`.
///
/// Snapshots are the states that get queried, checkpointed, and reconciled over time.
pub trait Snapshot: Send + Sync + Clone + Debug + PartialEq + Eq {
    /// The event type that mutates this snapshot.
    type Event: Event + ApplyEvent<Self>;

    /// Returns the logical snapshot id.
    fn id(&self) -> u128;
    /// Returns the snapshot time.
    fn time(&self) -> i64;
    /// Updates the snapshot time after replay or query.
    fn set_time(&mut self, time: i64);
    /// Returns a conservative upper-bound estimate for memory accounting.
    fn conservative_size(&self) -> u64;
    /// Builds the initial snapshot state for a snapshot id from its first event.
    fn from_event(event: &Self::Event) -> Self;
}

/// Marker trait for the generated or user-defined snapshot lane enum.
pub trait SnapshotLanes: Snapshot {}

/// A time-stamped input that can be routed through `contime`.
pub trait Event: Send + Sync + Debug {
    /// Returns the event id used for ordering and duplicate detection.
    fn id(&self) -> u128;
    /// Returns the event time.
    fn time(&self) -> i64;
    /// Returns a conservative upper-bound estimate for memory accounting.
    fn conservative_size(&self) -> u64;
}

/// Applies an event to a snapshot type.
pub trait ApplyEvent<S>: Event
where
    S: Snapshot,
{
    /// Returns the snapshot id affected by this event.
    fn snapshot_id(&self) -> u128;
    /// Mutates the snapshot in place.
    fn apply_to(&self, snapshot: &mut S);
}

/// Marker trait for the generated or user-defined event lane enum.
///
/// `snapshots()` returns the initial snapshots that should exist when this event creates
/// history for a snapshot id for the first time.
pub trait EventLanes<SL: SnapshotLanes>: Event + ApplyEvent<SL> + Clone {
    fn snapshots(&self) -> Vec<SL>;
}
