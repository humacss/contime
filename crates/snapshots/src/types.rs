/// The requested time cannot be reconstructed from retained checkpoint history.
///
/// Returned for a target before the horizon, even if an older checkpoint is
/// still physically present, or when no valid starting checkpoint exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoCheckpoint;

impl std::fmt::Display for NoCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("no valid checkpoint at or before the requested time")
    }
}
impl std::error::Error for NoCheckpoint {}

/// Materialized snapshot state and canonical application progress.
///
/// Completed checkpoints represent whole timestamp buckets. The initial
/// checkpoint instead precedes all supplied events, including those at its own
/// timestamp, and is identified by a zero event count.
#[derive(Clone, Debug)]
pub struct Checkpoint<S>
where
    S: Snapshot,
{
    /// Consumer state cloned to reconstruct queries and subsequent checkpoints.
    pub snapshot: S,
    /// Cumulative number of canonical events completed since initial state.
    ///
    /// The application kernel counts all events, including those the application
    /// ignores, and saturates at [`u64::MAX`]. Pruning does not reset the count.
    pub history_event_count: u64,
}

/// A discrete timestamp with a checked immediate predecessor.
pub trait Timestamp: Clone + Default + Ord {
    /// Returns the immediately preceding representable timestamp, or `None` at
    /// the minimum. Forwarding retains state at this predecessor so events
    /// exactly at the new horizon remain eligible for application.
    fn previous(&self) -> Option<Self>;
}

/// Borrowed access to canonically ordered events and physical history cleanup.
///
/// Iteration must yield increasing timestamps, with a deterministic order inside
/// each timestamp and no repeated event identities. Identity and tie-breaking
/// are defined by the implementation. The snapshot kernel relies on this order
/// to apply complete timestamp batches without sorting or deduplication.
pub trait EventStore {
    /// Ordered timestamp shared by the event store and its snapshot state.
    type Time: Clone + Default + Ord;
    /// Retained event value borrowed during snapshot reconstruction.
    type Event: Event<Time = Self::Time>;
    /// Canonical borrowed cursor, including every eligible event exactly once.
    type Iter<'a>: Iterator<Item = &'a Self::Event>
    where
        Self: 'a,
        Self::Time: 'a,
        Self::Event: 'a;

    /// Iterates canonically strictly after the complete timestamp `boundary`,
    /// or from the beginning when it is absent.
    fn iter_after(&self, boundary: Option<&Self::Time>) -> Self::Iter<'_>;

    /// Removes events strictly before the horizon, retaining events at it.
    /// Relative order and identity of retained events must remain unchanged;
    /// repeated cleanup at the same horizon must be harmless.
    fn prune_before(&mut self, horizon: &Self::Time);
}

/// Outcome of canonical event admission. Only `Inserted` changes history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insert {
    /// The event was retained and affected checkpoints must be invalidated.
    Inserted,
    /// An event with this identity is already retained; history is unchanged.
    Duplicate,
    /// The event is too old for retained history; history is unchanged.
    BeforeHorizon,
}

/// Optional insertion support for consumer-owned canonical event storage.
pub trait InsertEventStore: EventStore {
    /// Preserve canonical order and existing events. Duplicate or rejected
    /// admissions must leave history unchanged; identity is consumer-defined.
    fn insert(&mut self, event: Self::Event) -> Insert;
}

/// Consumer-owned state retained in checkpoints.
///
/// Cloning must preserve observable state while isolating later application
/// mutations from the retained original. Shared immutable data is permitted;
/// shared mutable state must not let query application modify retained state.
pub trait Snapshot: Clone {
    /// Ordered timestamp used for checkpoint validity and retained-history bounds.
    type Time: Clone + Default + Ord;

    /// Timestamp through which all canonical events have been processed.
    /// The initial snapshot is pre-event state, even at its own timestamp.
    fn time(&self) -> &Self::Time;

    /// Records completion of a whole timestamp batch, even if its events were filtered.
    fn set_time(&mut self, time: Self::Time);
}

/// Timestamp access used to group events into complete application batches.
///
/// This contract is independent of any event-storage representation or identity
/// scheme. [`EventStore`] supplies the stronger ordering requirements.
pub trait Event {
    /// Timestamp compared for equality when collecting one application batch.
    type Time: PartialEq;
    /// Returns the stable timestamp of this retained event.
    fn time(&self) -> Self::Time;
}

/// Access to the timestamp following the current complete event batch.
///
/// Read the value only after consuming the current batch iterator to completion.
/// A reported timestamp is later than the current batch. Availability depends on
/// the supplied playback range: `None` does not establish that all retained or
/// future history is exhausted. [`crate::SnapshotStore::process_until`] also exposes
/// the next playback interval when its boundary is reached; query and forwarding
/// do not provide that extra interval lookahead.
pub struct BatchLookahead<'a, T> {
    next_event_time: Option<&'a std::cell::RefCell<Option<Option<T>>>>,
    fallback: Option<&'a std::cell::RefCell<Option<Option<T>>>>,
}

impl<T> Clone for BatchLookahead<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for BatchLookahead<'_, T> {}

impl<'a, T> BatchLookahead<'a, T> {
    pub(crate) const fn new(next_event_time: &'a std::cell::RefCell<Option<Option<T>>>) -> Self {
        Self { next_event_time: Some(next_event_time), fallback: None }
    }

    /// Creates lookahead for a manually assembled batch with no following time.
    pub const fn unavailable() -> Self {
        Self { next_event_time: None, fallback: None }
    }

    pub(crate) const fn with_fallback(next_event_time: &'a std::cell::RefCell<Option<Option<T>>>, fallback: Self) -> Self {
        Self { next_event_time: Some(next_event_time), fallback: fallback.next_event_time }
    }

    /// Clones the next available timestamp, or returns `None` when none is exposed.
    /// Lookahead created with [`Self::unavailable`] always returns `None`.
    ///
    /// # Panics
    ///
    /// Panics for an active batch if its iterator has not been exhausted. Dropping
    /// the batch iterator drains it; reading lookahead while it is only partially
    /// consumed still violates this contract.
    pub fn next_event_time(self) -> Option<T>
    where
        T: Clone,
    {
        let next_event_time = self.next_event_time?;
        match next_event_time.borrow().as_ref().expect("the current event batch must be consumed before reading its lookahead").clone() {
            Some(time) => Some(time),
            None => self.fallback.and_then(|fallback| fallback.borrow().as_ref().cloned().flatten()),
        }
    }
}

/// Consumer-defined application of one complete timestamp batch.
///
/// Equivalent checkpoint state and canonical input must produce deterministic
/// results. External effects belong to the consumer; the same events can be
/// applied again during reconstruction. Query contexts must suppress effects
/// that should occur only during processing.
pub trait Apply<S, C = ()>: Event + Sized {
    /// Applies the nonempty iterator of events sharing one timestamp to `snapshot`.
    ///
    /// When `S` is [`Checkpoint`], the kernel records the completed timestamp and
    /// increments the raw event count after this call returns successfully. It
    /// drains unread events, so ignoring events still completes their timestamp.
    /// Consume or drop the iterator before reading `lookahead`.
    ///
    /// A panic propagates without transactional rollback of application mutations
    /// or effects. Completion metadata is not advanced for the failed batch.
    fn apply<'a>(snapshot: &mut S, events: impl Iterator<Item = &'a Self>, lookahead: BatchLookahead<'_, Self::Time>, context: &C)
    where
        Self: 'a;
}

/// Shared fixtures for unit tests and application benchmarks only.
#[cfg(test)]
pub(crate) mod testing {
    use super::{Apply, BatchLookahead, Checkpoint, Event, EventStore, Snapshot, Timestamp};

    pub type Time = u64;

    impl Timestamp for Time {
        fn previous(&self) -> Option<Self> {
            self.checked_sub(1)
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct TestSnapshot {
        pub time: Time,
        pub sum: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct TestEvent(pub Time, pub u64);
    #[derive(Default)]
    pub struct TestEventStore(pub Vec<TestEvent>);

    impl Snapshot for TestSnapshot {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.time
        }
        fn set_time(&mut self, time: Time) {
            self.time = time;
        }
    }

    impl Event for TestEvent {
        type Time = Time;
        fn time(&self) -> Time {
            self.0
        }
    }

    impl EventStore for TestEventStore {
        type Time = Time;
        type Event = TestEvent;
        type Iter<'a> = std::slice::Iter<'a, TestEvent>;
        fn iter_after(&self, boundary: Option<&Time>) -> Self::Iter<'_> {
            let start = boundary.map_or(0, |time| self.0.partition_point(|event| event.0 <= *time));
            self.0[start..].iter()
        }
        fn prune_before(&mut self, horizon: &Time) {
            let end = self.0.partition_point(|event| event.0 < *horizon);
            self.0.drain(..end);
        }
    }

    impl Apply<Checkpoint<TestSnapshot>> for TestEvent {
        fn apply<'a>(
            checkpoint: &mut Checkpoint<TestSnapshot>,
            events: impl Iterator<Item = &'a Self>,
            _: BatchLookahead<'_, Time>,
            _: &(),
        ) {
            checkpoint.snapshot.sum += events.map(|event| event.1).sum::<u64>();
        }
    }

    impl Apply<Checkpoint<TestSnapshot>, u64> for TestEvent {
        fn apply<'a>(
            checkpoint: &mut Checkpoint<TestSnapshot>,
            events: impl Iterator<Item = &'a Self>,
            _: BatchLookahead<'_, Time>,
            context: &u64,
        ) {
            checkpoint.snapshot.sum += events.map(|event| event.1).sum::<u64>() + context;
        }
    }

    #[test]
    fn sum_fixture_preserves_checkpoint_metadata() {
        let expected_sum = 12;
        let expected_context_sum = 22;
        let expected_time = 10;
        let expected_count = 7;

        let events = [TestEvent(20, 3), TestEvent(20, 4)];
        let initial = Checkpoint { snapshot: TestSnapshot { time: expected_time, sum: 5 }, history_event_count: expected_count };
        let mut plain = initial.clone();
        let mut contextual = initial;
        let context = 10u64;

        TestEvent::apply(&mut plain, events.iter(), BatchLookahead::unavailable(), &());
        TestEvent::apply(&mut contextual, events.iter(), BatchLookahead::unavailable(), &context);

        assert_eq!(plain.snapshot.sum, expected_sum);
        assert_eq!(contextual.snapshot.sum, expected_context_sum);
        assert_eq!(plain.snapshot.time, expected_time);
        assert_eq!(contextual.snapshot.time, expected_time);
        assert_eq!(plain.history_event_count, expected_count);
        assert_eq!(contextual.history_event_count, expected_count);
    }
}
