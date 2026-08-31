use std::collections::{BTreeMap, VecDeque};

use crate::EventHistory;
use crate::{Event, EventHistoryIter, EventKey};

impl<E> Default for EventHistory<E>
where
    E: Event,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<E> EventHistory<E>
where
    E: Event,
{
    /// Creates an empty event history.
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    /// Creates an empty history with capacity for the expected ordered events
    /// and retained IDs.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ordered: VecDeque::with_capacity(capacity),
            late: BTreeMap::new(),
            retained_ids: ahash::AHashSet::with_capacity(capacity),
            dirty_time: E::Time::default(),
        }
    }

    /// Returns the number of retained events.
    pub fn len(&self) -> usize {
        self.ordered.len() + self.late.len()
    }

    /// Returns whether the history contains no events.
    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty() && self.late.is_empty()
    }

    /// Returns `(ordered, late)` retained-event counts.
    pub fn storage_counts(&self) -> (usize, usize) {
        (self.ordered.len(), self.late.len())
    }

    /// Returns the latest canonical key in either store.
    pub fn latest_key(&self) -> Option<&EventKey<E::Time>> {
        match (self.ordered.back().map(|(key, _event)| key), self.late.last_key_value().map(|(key, _event)| key)) {
            (Some(ordered), Some(late)) => Some(ordered.max(late)),
            (Some(key), None) | (None, Some(key)) => Some(key),
            (None, None) => None,
        }
    }

    /// Returns the earliest timestamp from which replay must begin.
    pub fn dirty_time(&self) -> &E::Time {
        &self.dirty_time
    }

    /// Marks all retained events as replayed.
    ///
    /// A clean non-empty history keeps its latest timestamp as a conservative
    /// boundary so another event at that timestamp replays the complete bucket.
    /// An empty history returns to its zero timestamp.
    pub fn mark_replayed(&mut self) {
        self.dirty_time = self.latest_key().map(|key| key.time.clone()).unwrap_or_default();
    }

    /// Iterates retained events in canonical `(time, event ID)` order.
    pub fn iter(&self) -> EventHistoryIter<'_, E> {
        EventHistoryIter::new(self.ordered.iter(), self.late.iter())
    }
}

#[cfg(test)]
mod tests {
    use criterion::{BatchSize, Criterion};
    use std::hint::black_box;

    use super::EventHistory;
    use crate::{Event, EventKey};

    struct TestEvent {
        id: u128,
        time: i64,
    }

    impl Event for TestEvent {
        type Time = i64;

        fn event_id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> Self::Time {
            self.time
        }
    }

    fn event(id: u128, time: i64) -> TestEvent {
        TestEvent { id, time }
    }

    fn populated_history() -> EventHistory<TestEvent> {
        let mut history = EventHistory::with_capacity(3);
        history.ordered.push_back((EventKey { time: 10, event_id: 1 }, event(1, 10)));
        history.ordered.push_back((EventKey { time: 30, event_id: 3 }, event(3, 30)));
        history.late.insert(EventKey { time: 20, event_id: 2 }, event(2, 20));
        history.retained_ids.extend([1, 2, 3]);
        history
    }

    #[test]
    fn a_new_history_is_empty() {
        let history = EventHistory::<TestEvent>::new();

        assert!(history.is_empty());
        assert_eq!(history.len(), 0);
        assert_eq!(history.storage_counts(), (0, 0));
        assert_eq!(history.latest_key(), None);
        assert_eq!(history.dirty_time(), &0);
    }

    #[test]
    fn metadata_reflects_both_internal_stores() {
        let history = populated_history();

        assert!(!history.is_empty());
        assert_eq!(history.len(), 3);
        assert_eq!(history.storage_counts(), (2, 1));
        assert_eq!(history.latest_key(), Some(&EventKey { time: 30, event_id: 3 }));
    }

    #[test]
    fn marking_replayed_moves_dirty_time_to_the_latest_retained_time() {
        let mut history = EventHistory::new();
        history.insert(event(3, 30));
        history.insert(event(1, 10));
        assert_eq!(history.dirty_time(), &10);

        history.mark_replayed();

        assert_eq!(history.dirty_time(), &30);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_history() {
        let mut criterion = Criterion::default();

        criterion.bench_function("events/history/create_with_capacity_1000", |bencher| {
            bencher.iter_batched(|| (), |()| black_box(EventHistory::<TestEvent>::with_capacity(1_000)), BatchSize::SmallInput);
        });

        criterion.bench_function("events/history/1000_metadata_reads", |bencher| {
            let history = populated_history();
            bencher.iter(|| {
                for _ in 0..1_000 {
                    black_box(history.len());
                    black_box(history.storage_counts());
                    black_box(history.latest_key());
                }
            });
        });

        criterion.final_summary();
    }
}
