use std::fmt::Debug;

use crate::ContimeTime;

/// A point-in-time state for one logical entity in `contime`.
///
/// Snapshots are the states that get queried and checkpointed over time.
pub trait Snapshot: Send + Sync + Clone + Debug + PartialEq + Eq {
    /// Ordered time type shared by this snapshot and all of its events.
    type Time: ContimeTime;
    /// The event type that mutates this snapshot.
    type Event: Event<Time = Self::Time> + Clone + PartialEq + Eq;

    /// Returns the logical snapshot id.
    fn id(&self) -> u128;
    /// Returns the snapshot time.
    fn time(&self) -> Self::Time;
    /// Updates the snapshot time after replay or query.
    fn set_time(&mut self, time: Self::Time);
    /// Returns a conservative upper-bound estimate for memory accounting.
    fn conservative_size(&self) -> u64;
    /// Builds the initial snapshot state for a snapshot id from its first event.
    fn from_event(event: &Self::Event) -> Self;
}

/// Routes a concrete event to the snapshot instance it affects.
pub trait SnapshotEvent<S>: Event<Time = S::Time>
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
    E: Event<Time = Self::Time>,
{
    /// Builds the initial snapshot state for a snapshot id from one routed event.
    fn seed_from_event(event: &E) -> Self;
}

impl<S, E> SeedSnapshot<E> for S
where
    S: Snapshot<Event = E>,
    E: Event<Time = S::Time>,
{
    fn seed_from_event(event: &E) -> Self {
        S::from_event(event)
    }
}

/// Marker trait for the generated or user-defined snapshot lane enum.
pub trait SnapshotLanes: Snapshot {}

/// A time-stamped input that can be routed through `contime`.
pub trait Event: Send + Sync + Debug {
    /// Ordered time type used by this event.
    type Time: ContimeTime;

    /// Returns the event id used for ordering and duplicate detection.
    fn id(&self) -> u128;
    /// Returns the event time.
    fn time(&self) -> Self::Time;
    /// Returns a conservative upper-bound estimate for memory accounting.
    fn conservative_size(&self) -> u64;
}

/// Applies one bucket of events with the same complete ordered time to a snapshot.
///
/// Events are supplied in deterministic event-id order, but implementations
/// must treat that ordering as transport determinism rather than domain
/// priority.
#[derive(Debug)]
pub struct ApplyBatch<'a, E: Event> {
    pub snapshot_id: u128,
    pub time: E::Time,
    pub events: &'a [&'a E],
}

impl<'a, E: Event> Clone for ApplyBatch<'a, E> {
    fn clone(&self) -> Self {
        Self { snapshot_id: self.snapshot_id, time: self.time.clone(), events: self.events }
    }
}

pub trait ApplyEvents: Snapshot {
    /// Mutates the snapshot with all events for one routed `time` bucket.
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>);
}

/// Marker trait for the generated or user-defined event lane enum.
///
/// `snapshots()` returns the initial snapshots that should exist when this event creates
/// history for a snapshot id for the first time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedSnapshot<SL> {
    pub snapshot_id: u128,
    pub initial_snapshot: SL,
}

pub trait EventLanes<SL: SnapshotLanes, C = ()>: Event<Time = SL::Time> + Clone + PartialEq + Eq
where
    SL: ApplyEvents,
{
    fn snapshots(&self) -> Vec<SL>;

    fn routed_snapshots(&self) -> Vec<RoutedSnapshot<SL>> {
        self.snapshots().into_iter().map(|snapshot| RoutedSnapshot { snapshot_id: snapshot.id(), initial_snapshot: snapshot }).collect()
    }
}
