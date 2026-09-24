/// No valid checkpoint exists at or before the requested time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoCheckpoint;

impl std::fmt::Display for NoCheckpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("no valid checkpoint at or before the requested time")
    }
}
impl std::error::Error for NoCheckpoint {}

/// Materialized state at a complete timestamp boundary.
#[derive(Clone, Debug)]
pub struct Checkpoint<S>
where
    S: Snapshot,
{
    pub snapshot: S,
    pub history_event_count: u64,
}

/// A discrete timestamp with a checked immediate predecessor.
pub trait Timestamp: Clone + Default + Ord {
    fn previous(&self) -> Option<Self>;
}

/// Borrowed access to canonical events. Insertion and physical cleanup belong
/// to the owning consumer, not checkpoint commit policies.
pub trait EventStore {
    type Time: Clone + Default + Ord;
    type Event: Event<Time = Self::Time>;
    type Iter<'a>: Iterator<Item = &'a Self::Event>
    where
        Self: 'a,
        Self::Time: 'a,
        Self::Event: 'a;

    /// Iterates canonically strictly after the complete timestamp `boundary`,
    /// or from the beginning when it is absent.
    fn iter_after(&self, boundary: Option<&Self::Time>) -> Self::Iter<'_>;

    /// Removes events strictly before the horizon, retaining events at it.
    fn prune_before(&mut self, horizon: &Self::Time);
}

/// Outcome of canonical event admission. Only `Inserted` changes history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insert {
    Inserted,
    Duplicate,
    BeforeHorizon,
}

/// Optional insertion support for consumer-owned canonical event storage.
pub trait InsertEventStore: EventStore {
    /// Preserve canonical order and existing events. Duplicate or rejected
    /// admissions must leave history unchanged; identity is consumer-defined.
    fn insert(&mut self, event: Self::Event) -> Insert;
}

/// Consumer-owned state retained in checkpoints.
pub trait Snapshot: Clone {
    type Time: Clone + Default + Ord;

    /// Timestamp through which all canonical events have been processed.
    fn time(&self) -> &Self::Time;

    /// Records completion of a whole timestamp batch, even if its events were filtered.
    fn set_time(&mut self, time: Self::Time);
}

/// Time used for timestamp batches and checkpoint boundaries.
/// Core adapters may implement this independently of their event-store contract.
pub trait Event {
    type Time: PartialEq;
    fn time(&self) -> Self::Time;
}

/// Consumer-defined application of one complete timestamp batch.
/// Mutation and any application hooks belong entirely to this implementation.
pub trait Apply<S, C = ()>: Event + Sized {
    fn apply<'a>(snapshot: &mut S, events: impl Iterator<Item = &'a Self>, context: &C)
    where
        Self: 'a;
}

/// Shared fixtures for unit tests and application benchmarks only.
#[cfg(test)]
pub(crate) mod testing {
    use super::{Apply, Checkpoint, Event, EventStore, Snapshot, Timestamp};

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
        fn apply<'a>(checkpoint: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, _: &()) {
            checkpoint.snapshot.sum += events.map(|event| event.1).sum::<u64>();
        }
    }

    impl Apply<Checkpoint<TestSnapshot>, u64> for TestEvent {
        fn apply<'a>(checkpoint: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, context: &u64) {
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

        TestEvent::apply(&mut plain, events.iter(), &());
        TestEvent::apply(&mut contextual, events.iter(), &context);

        assert_eq!(plain.snapshot.sum, expected_sum);
        assert_eq!(contextual.snapshot.sum, expected_context_sum);
        assert_eq!(plain.snapshot.time, expected_time);
        assert_eq!(contextual.snapshot.time, expected_time);
        assert_eq!(plain.history_event_count, expected_count);
        assert_eq!(contextual.history_event_count, expected_count);
    }
}
