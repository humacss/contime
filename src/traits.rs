use std::fmt::Debug;

/// A point-in-time state for one logical entity in `contime`.
///
/// Snapshots are the states that get queried, checkpointed, and reconciled over time.
pub trait Snapshot: Send + Sync + Clone + Debug + PartialEq + Eq {
    /// The event type that mutates this snapshot.
    type Event: Event + Clone + PartialEq + Eq;

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

/// Routes a concrete event to the snapshot instance it affects.
pub trait SnapshotEvent<S>: Event
where
    S: Snapshot,
{
    /// Returns the logical snapshot id affected by this event.
    fn snapshot_id(&self) -> u128;
}

/// Seeds the first materialized snapshot state for a concrete routed event type.
///
/// This is separate from [`Snapshot::from_event`] so merged host lane enums can
/// initialize foreign snapshots without needing foreign `From<E> for
/// <Snapshot as Snapshot>::Event` impls, which violate Rust's orphan rules.
pub trait SeedSnapshot<E>: Snapshot
where
    E: Event,
{
    /// Builds the initial snapshot state for a snapshot id from one routed event.
    fn seed_from_event(event: &E) -> Self;
}

impl<S, E> SeedSnapshot<E> for S
where
    S: Snapshot<Event = E>,
    E: Event,
{
    fn seed_from_event(event: &E) -> Self {
        S::from_event(event)
    }
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

/// Applies one same-millisecond bucket of events to a snapshot.
///
/// Events are supplied in deterministic event-id order, but implementations
/// must treat that ordering as transport determinism rather than domain
/// priority.
#[derive(Debug)]
pub struct ApplyBatch<'a, E> {
    pub snapshot_id: u128,
    pub time: i64,
    pub events: &'a [E],
}

impl<'a, E> Clone for ApplyBatch<'a, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, E> Copy for ApplyBatch<'a, E> {}

pub trait ApplyEvents: Snapshot {
    /// Mutates the snapshot with all events for one routed `time` bucket.
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>);
}

/// Runs context-aware work after one event bucket applies to a snapshot.
pub trait AfterApplyEvents<C = ()>: Snapshot {
    /// Runs after [`ApplyEvents::apply_events`] against the final post-bucket
    /// snapshot.
    fn after_apply_events(&self, _batch: ApplyBatch<'_, Self::Event>, _context: &mut C) {}
}

/// Marker trait for the generated or user-defined event lane enum.
///
/// `snapshots()` returns the initial snapshots that should exist when this event creates
/// history for a snapshot id for the first time.
pub trait EventLanes<SL: SnapshotLanes, C = ()>: Event + Clone
where
    SL: ApplyEvents + AfterApplyEvents<C>,
{
    fn snapshots(&self) -> Vec<SL>;
}
