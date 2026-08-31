use crate::{Event, EventHistory, EventKey, Insert};

impl Insert {
    /// Returns whether canonical history changed.
    pub const fn changed(&self) -> bool {
        matches!(self, Self::Inserted)
    }
}

impl<E> EventHistory<E>
where
    E: Event,
{
    /// Retains one event unless its ID is already present.
    ///
    /// In-order events append to the ordered deque. Events before the current
    /// tail enter the late-event tree. Identity is independent of timestamp:
    /// a retained event ID always makes a later insertion a no-op.
    pub fn insert(&mut self, event: E) -> Insert {
        let event_id = event.event_id();
        if !self.retained_ids.insert(event_id) {
            return Insert::Duplicate;
        }

        let was_empty = self.is_empty();
        let key = EventKey::from_event(&event);
        if was_empty || key.time < self.dirty_time {
            self.dirty_time = key.time.clone();
        }

        if self.ordered.back().is_none_or(|(latest, _event)| key > *latest) {
            self.ordered.push_back((key, event));
        } else {
            let previous = self.late.insert(key, event);
            debug_assert!(previous.is_none(), "a unique retained event ID produced a duplicate canonical key");
        }

        Insert::Inserted
    }
}

#[cfg(test)]
mod tests {
    use criterion::{BatchSize, Criterion};
    use std::hint::black_box;

    use crate::{Event, EventHistory, EventKey, Insert};

    #[derive(Clone)]
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
    fn history_retains_the_selected_event_ownership_type_directly() {
        let mut history = EventHistory::new();

        history.insert(TestEvent { id: 7, time: 11 });

        let (_key, retained) = history.iter().next().unwrap();
        assert_eq!(retained.id, 7);
        assert_eq!(retained.time, 11);
    }

    #[test]
    fn an_event_after_the_tail_uses_ordered_storage() {
        let mut history = EventHistory::new();

        let inserted = history.insert(event(1, 10));

        assert_eq!(inserted, Insert::Inserted);
        assert!(inserted.changed());
        assert_eq!(history.storage_counts(), (1, 0));
        assert_eq!(history.dirty_time(), &10);
    }

    #[test]
    fn an_event_before_the_tail_uses_late_storage() {
        let mut history = EventHistory::new();
        history.insert(event(2, 20));

        let inserted = history.insert(event(1, 10));

        assert_eq!(inserted, Insert::Inserted);
        assert_eq!(history.storage_counts(), (1, 1));
    }

    #[test]
    fn a_retained_id_is_a_duplicate_even_when_time_changes() {
        let mut history = EventHistory::new();
        history.insert(event(1, 10));

        let duplicate = history.insert(event(1, 100));

        assert_eq!(duplicate, Insert::Duplicate);
        assert!(!duplicate.changed());
        assert_eq!(history.len(), 1);
        assert_eq!(history.latest_key(), Some(&EventKey { time: 10, event_id: 1 }));
    }

    #[test]
    fn a_late_event_moves_dirty_time_back_but_a_duplicate_does_not() {
        let mut history = EventHistory::new();
        history.insert(event(3, 30));
        history.mark_replayed();

        history.insert(event(1, 10));
        assert_eq!(history.dirty_time(), &10);

        history.insert(event(1, 5));
        assert_eq!(history.dirty_time(), &10);
    }

    fn ordered_fixture() -> (EventHistory<TestEvent>, Vec<TestEvent>) {
        let events = (0..1_000).map(|value| event(value, value as i64)).collect();
        (EventHistory::with_capacity(1_000), events)
    }

    fn late_fixture() -> (EventHistory<TestEvent>, Vec<TestEvent>) {
        let mut history = EventHistory::with_capacity(1_001);
        history.insert(event(10_000, 10_000));
        let events = (0..1_000).map(|value| event(value, value as i64)).collect();
        (history, events)
    }

    fn duplicate_fixture() -> (EventHistory<TestEvent>, Vec<TestEvent>) {
        let events = (0..1_000).map(|value| event(value, value as i64)).collect::<Vec<_>>();
        let mut history = EventHistory::with_capacity(1_000);
        for event in &events {
            history.insert(event.clone());
        }
        (history, events)
    }

    fn benchmark_fixture(criterion: &mut Criterion, name: &str, fixture: fn() -> (EventHistory<TestEvent>, Vec<TestEvent>)) {
        criterion.bench_function(&format!("events/insert/1000_{name}"), |bencher| {
            bencher.iter_batched(
                fixture,
                |(mut history, events)| {
                    for event in events {
                        black_box(history.insert(event));
                    }
                    black_box(history)
                },
                BatchSize::LargeInput,
            );
        });
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_insert() {
        let mut criterion = Criterion::default();

        benchmark_fixture(&mut criterion, "ordered", ordered_fixture);
        benchmark_fixture(&mut criterion, "late", late_fixture);
        benchmark_fixture(&mut criterion, "duplicate", duplicate_fixture);

        criterion.final_summary();
    }
}
