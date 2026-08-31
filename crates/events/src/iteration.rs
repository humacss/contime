use std::collections::{btree_map, vec_deque};
use std::ops::Bound;

use crate::{Event, EventHistoryIter, EventHistoryRangeIter, EventKey};

impl<'a, E> EventHistoryIter<'a, E>
where
    E: Event,
{
    pub(crate) fn new(ordered: vec_deque::Iter<'a, (EventKey<E::Time>, E)>, late: btree_map::Iter<'a, EventKey<E::Time>, E>) -> Self {
        Self { ordered: ordered.peekable(), late: late.peekable() }
    }
}

impl<E> crate::EventHistory<E>
where
    E: Event,
{
    /// Iterates canonically from the dirty timestamp, including its complete
    /// same-timestamp event bucket.
    pub fn iter_from_dirty(&self) -> EventHistoryRangeIter<'_, E> {
        let boundary = EventKey { time: self.dirty_time.clone(), event_id: u128::MIN };
        let ordered_start = self.ordered.partition_point(|(key, _event)| key < &boundary);
        EventHistoryRangeIter::new(self.ordered.range(ordered_start..), self.late.range(boundary..))
    }

    /// Iterates canonically strictly after an exact `(time, event ID)` key.
    pub fn iter_after(&self, boundary: &EventKey<E::Time>) -> EventHistoryRangeIter<'_, E> {
        let ordered_start = self.ordered.partition_point(|(key, _event)| key <= boundary);
        EventHistoryRangeIter::new(
            self.ordered.range(ordered_start..),
            self.late.range((Bound::Excluded(boundary.clone()), Bound::Unbounded)),
        )
    }
}

impl<'a, E> EventHistoryRangeIter<'a, E>
where
    E: Event,
{
    pub(crate) fn new(ordered: vec_deque::Iter<'a, (EventKey<E::Time>, E)>, late: btree_map::Range<'a, EventKey<E::Time>, E>) -> Self {
        Self { ordered: ordered.peekable(), late: late.peekable() }
    }
}

impl<'a, E> Iterator for EventHistoryRangeIter<'a, E>
where
    E: Event,
{
    type Item = (&'a EventKey<E::Time>, &'a E);

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        match (self.ordered.peek(), self.late.peek()) {
            (Some((ordered_key, _ordered_event)), Some((late_key, _late_event))) if ordered_key < *late_key => {
                self.ordered.next().map(|(key, event)| (key, event))
            }
            (Some(_), Some(_)) | (None, Some(_)) => self.late.next().map(|(key, event)| (key, event)),
            (Some(_), None) => self.ordered.next().map(|(key, event)| (key, event)),
            (None, None) => None,
        }
    }
}

impl<'a, E> Iterator for EventHistoryIter<'a, E>
where
    E: Event,
{
    type Item = (&'a EventKey<E::Time>, &'a E);

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        match (self.ordered.peek(), self.late.peek()) {
            (Some((ordered_key, _ordered_event)), Some((late_key, _late_event))) if ordered_key < *late_key => {
                self.ordered.next().map(|(key, event)| (key, event))
            }
            (Some(_), Some(_)) | (None, Some(_)) => self.late.next().map(|(key, event)| (key, event)),
            (Some(_), None) => self.ordered.next().map(|(key, event)| (key, event)),
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use criterion::Criterion;
    use std::hint::black_box;

    use crate::{Event, EventHistory};

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

    #[test]
    fn iteration_merges_ordered_and_late_events_by_time_then_id() {
        let mut history = EventHistory::new();
        history.insert(event(30, 10));
        history.insert(event(50, 20));
        history.insert(event(20, 10));
        history.insert(event(40, 15));

        let keys = history.iter().map(|(key, _event)| (key.time, key.event_id)).collect::<Vec<_>>();

        assert_eq!(keys, vec![(10, 20), (10, 30), (15, 40), (20, 50)]);
    }

    #[test]
    fn an_empty_history_has_no_iteration_items() {
        let history = EventHistory::<TestEvent>::new();

        assert!(history.iter().next().is_none());
    }

    #[test]
    fn dirty_iteration_includes_the_complete_first_timestamp_bucket() {
        let mut history = EventHistory::new();
        history.insert(event(20, 10));
        history.insert(event(40, 20));
        history.mark_replayed();
        history.insert(event(10, 10));

        let keys = history.iter_from_dirty().map(|(key, _event)| (key.time, key.event_id)).collect::<Vec<_>>();

        assert_eq!(keys, vec![(10, 10), (10, 20), (20, 40)]);
    }

    #[test]
    fn boundary_iteration_starts_strictly_after_time_and_event_id() {
        let mut history = EventHistory::new();
        history.insert(event(20, 10));
        history.insert(event(40, 20));
        history.insert(event(10, 10));
        history.insert(event(30, 15));

        let keys = history
            .iter_after(&crate::EventKey { time: 10, event_id: 10 })
            .map(|(key, _event)| (key.time, key.event_id))
            .collect::<Vec<_>>();

        assert_eq!(keys, vec![(10, 20), (15, 30), (20, 40)]);
    }

    fn history_with_late_percentage(late_percentage: usize) -> EventHistory<TestEvent> {
        let late_count = late_percentage * 10;
        let mut history = EventHistory::with_capacity(1_000);

        for value in late_count..1_000 {
            history.insert(event(value as u128, value as i64));
        }
        for value in 0..late_count {
            history.insert(event(value as u128, value as i64));
        }

        history
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_iteration() {
        let mut criterion = Criterion::default();

        let ordered_history = history_with_late_percentage(0);
        criterion.bench_function("events/iteration/1000_events/ordered_storage_direct", |bencher| {
            bencher.iter(|| {
                let mut count = 0;
                for entry in &ordered_history.ordered {
                    black_box(entry);
                    count += 1;
                }
                black_box(count)
            });
        });

        criterion.bench_function("events/iteration/1000_events/from_dirty", |bencher| {
            bencher.iter(|| {
                let mut count = 0;
                for entry in ordered_history.iter_from_dirty() {
                    black_box(entry);
                    count += 1;
                }
                black_box(count)
            });
        });

        for late_percentage in [0, 10, 50] {
            let history = history_with_late_percentage(late_percentage);
            criterion.bench_function(&format!("events/iteration/1000_events/{late_percentage}_percent_late"), |bencher| {
                bencher.iter(|| {
                    let mut count = 0;
                    for entry in history.iter() {
                        black_box(entry);
                        count += 1;
                    }
                    black_box(count)
                });
            });
        }

        criterion.final_summary();
    }
}
