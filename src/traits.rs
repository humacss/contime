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
    E: Event + ApplyEvent<S>,
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

/// Runs context-aware work after one event bucket applies to a snapshot.
pub trait AfterApplyEvent<S, C = ()>: ApplyEvent<S>
where
    S: Snapshot,
{
    /// Runs after [`ApplyEvent::apply_to`] against the actual post-apply
    /// snapshot.
    ///
    /// Context is runtime plumbing. Implementations must not mutate the
    /// snapshot and must keep side effects deterministic and nonblocking.
    fn after_apply(&self, _snapshot: &S, _context: &mut C) {}

    /// Runs once after a same-millisecond event bucket has been applied.
    ///
    /// The default preserves legacy behavior by invoking [`AfterApplyEvent::after_apply`]
    /// for every event against the final post-bucket snapshot.
    fn after_apply_events(snapshot: &S, events: &[Self], context: &mut C)
    where
        Self: Sized,
    {
        for event in events {
            event.after_apply(snapshot, context);
        }
    }
}

/// Applies one same-millisecond bucket of events to a snapshot.
///
/// Events are supplied in deterministic event-id order. Implementations may
/// override this when same-millisecond events need semantic merge behavior.
pub trait ApplyEvents<C = ()>: Snapshot {
    /// Mutates the snapshot with all events for `time`.
    fn apply_events(&mut self, time: i64, events: &[Self::Event]);

    /// Runs after [`ApplyEvents::apply_events`] against the final post-bucket
    /// snapshot.
    fn after_apply_events(&self, _time: i64, _events: &[Self::Event], _context: &mut C) {}
}

/// Marker trait for the generated or user-defined event lane enum.
///
/// `snapshots()` returns the initial snapshots that should exist when this event creates
/// history for a snapshot id for the first time.
pub trait EventLanes<SL: SnapshotLanes, C = ()>: Event + ApplyEvent<SL> + Clone
where
    SL: ApplyEvents<C>,
{
    fn snapshots(&self) -> Vec<SL>;
}
