use crate::contime_simulator::Random;
use std::iter::FusedIterator;

/// Finite event stream configuration with application-defined settings.
///
/// `settings` can describe snapshot population, distribution, event payloads,
/// logical duration, or any other inputs required by the two callbacks. The
/// callbacks interpret and validate those settings; the stream does not inspect
/// application types or enforce their routing semantics.
#[derive(Clone, Debug)]
pub struct StreamConfig<C> {
    /// Number of events to yield. Zero produces an empty stream.
    pub event_count: u64,
    /// Seed for independent event-creation and snapshot-assignment random streams.
    pub seed: u64,
    /// Application-specific inputs shared by both callbacks.
    pub settings: C,
}

/// Lazy event iterator driven by creation and snapshot-ID assignment callbacks.
///
/// Each `next()` creates one event and then mutates it with the assignment
/// callback before yielding it. No batches, clocks, submissions, queries,
/// threads, or allocations are introduced by the iterator itself.
///
/// The callbacks receive independent random streams so extra payload draws do
/// not change snapshot selection. Determinism also requires deterministic
/// callbacks and the same callback state. Creating a new stream restarts its
/// random sequences; consumers own event ID uniqueness across streams.
///
/// Callbacks may capture state and return any event type, including a `Result`.
/// Such a result is an ordinary item: failures do not stop this iterator or undo
/// callback changes. Panicking callbacks are not a recoverable stream operation.
pub struct EventStream<C, Make, Assign> {
    config: StreamConfig<C>,
    remaining: u64,
    make: Make,
    assign: Assign,
    event_random: Random,
    snapshot_random: Random,
}

impl<C, Make, Assign, E> EventStream<C, Make, Assign>
where
    Make: FnMut(&StreamConfig<C>, &mut Random) -> E,
    Assign: FnMut(&StreamConfig<C>, &mut Random, &mut E),
{
    /// Creates a stream without invoking either callback.
    ///
    /// `make(config, random)` creates an event. Then
    /// `assign(config, random, &mut event)` sets its snapshot-identifying fields.
    /// If event IDs are content-derived, compute them after assignment or derive
    /// them on demand from the completed event.
    pub fn new(config: StreamConfig<C>, make: Make, assign: Assign) -> Self {
        let remaining = config.event_count;
        let event_random = Random::new(config.seed);
        let snapshot_random = Random::new(config.seed ^ 0xd1b5_4a32_d192_ed03);
        Self { config, remaining, make, assign, event_random, snapshot_random }
    }
}

impl<C, Make, Assign, E> Iterator for EventStream<C, Make, Assign>
where
    Make: FnMut(&StreamConfig<C>, &mut Random) -> E,
    Assign: FnMut(&StreamConfig<C>, &mut Random, &mut E),
{
    type Item = E;

    fn next(&mut self) -> Option<E> {
        if self.remaining == 0 {
            return None;
        }
        let mut event = (self.make)(&self.config, &mut self.event_random);
        (self.assign)(&self.config, &mut self.snapshot_random, &mut event);
        self.remaining -= 1;
        Some(event)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match usize::try_from(self.remaining) {
            Ok(remaining) => (remaining, Some(remaining)),
            Err(_) => (usize::MAX, None),
        }
    }
}

impl<C, Make, Assign, E> FusedIterator for EventStream<C, Make, Assign>
where
    Make: FnMut(&StreamConfig<C>, &mut Random) -> E,
    Assign: FnMut(&StreamConfig<C>, &mut Random, &mut E),
{
}
