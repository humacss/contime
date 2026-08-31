use crate::{Event, EventHistory};

impl<E> EventHistory<E>
where
    E: Event,
{
    /// Clones retained events in canonical order over `[from, to)`.
    pub fn clone_between(&self, from: &E::Time, to: &E::Time) -> Vec<E>
    where
        E: Clone,
    {
        if from >= to {
            return Vec::new();
        }

        self.iter_from_time(from).take_while(|(key, _event)| &key.time < to).map(|(_key, event)| event.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::sync::Arc;

    use criterion::Criterion;

    use crate::{Event, EventHistory};

    #[derive(Clone, Debug, Eq, PartialEq)]
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
    fn clone_between_returns_owned_events_in_canonical_half_open_order() {
        let mut history = EventHistory::new();
        history.insert(event(30, 30));
        history.insert(event(10, 10));
        history.insert(event(20, 20));

        let result = history.clone_between(&10, &30);

        assert_eq!(result, vec![event(10, 10), event(20, 20)]);
    }

    #[test]
    fn cloned_results_survive_history_mutation_and_drop() {
        let mut history = EventHistory::new();
        history.insert(event(1, 10));
        let result = history.clone_between(&0, &20);

        history.insert(event(2, 15));
        drop(history);

        assert_eq!(result, vec![event(1, 10)]);
    }

    #[test]
    fn empty_or_reversed_ranges_return_no_events() {
        let mut history = EventHistory::new();
        history.insert(event(1, 10));

        assert!(history.clone_between(&10, &10).is_empty());
        assert!(history.clone_between(&20, &10).is_empty());
        assert!(EventHistory::<TestEvent>::new().clone_between(&0, &20).is_empty());
    }

    #[derive(Clone)]
    struct SharedEvent(Arc<TestEvent>);

    impl Event for SharedEvent {
        type Time = i64;

        fn event_id(&self) -> u128 {
            self.0.id
        }

        fn time(&self) -> Self::Time {
            self.0.time
        }
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_query() {
        let mut history = EventHistory::with_capacity(1_000);
        for id in 0..1_000 {
            history.insert(SharedEvent(Arc::new(event(id, id as i64))));
        }

        let mut criterion = Criterion::default();
        criterion.bench_function("events/query/clone_between/1000_events", |bencher| {
            bencher.iter(|| black_box(history.clone_between(black_box(&0), black_box(&1_000))));
        });
        criterion.final_summary();
    }
}
