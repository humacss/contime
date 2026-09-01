use crate::{Event, EventHistory, EventKey, PruneResult};

impl<E> EventHistory<E>
where
    E: Event,
{
    /// Advances the retained-history horizon and removes older events.
    pub fn prune_before(&mut self, horizon: &E::Time) -> PruneResult {
        if horizon <= &self.horizon {
            return PruneResult { removed_ordered: 0, removed_late: 0 };
        }
        self.horizon = horizon.clone();

        let boundary = EventKey { time: horizon.clone(), event_id: u128::MIN };
        let retained_late = self.late.split_off(&boundary);
        let removed_late = std::mem::replace(&mut self.late, retained_late);
        let removed_late_count = removed_late.len();
        for event in removed_late.into_values() {
            self.retained_ids.remove(&event.event_id());
        }

        let mut removed_ordered = 0;
        while self.ordered.front().is_some_and(|(key, _event)| key.time < *horizon) {
            let (_key, event) = self.ordered.pop_front().expect("the inspected front event exists");
            self.retained_ids.remove(&event.event_id());
            removed_ordered += 1;
        }

        if self.is_empty() {
            self.dirty_time = horizon.clone();
        }

        PruneResult { removed_ordered, removed_late: removed_late_count }
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::{BatchSize, Criterion};

    use crate::{Event, EventHistory, Insert, PruneResult};

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
    fn pruning_removes_both_stores_and_retains_the_boundary() {
        let mut history = EventHistory::with_horizon(0);
        history.insert(event(3, 30));
        history.insert(event(1, 10));
        history.insert(event(2, 20));

        let removed = history.prune_before(&20);

        assert_eq!(removed, PruneResult { removed_ordered: 0, removed_late: 1 });
        assert_eq!(history.iter().map(|(key, _)| key.time).collect::<Vec<_>>(), vec![20, 30]);
    }

    #[test]
    fn pruning_reaches_the_front_of_the_ordered_deque() {
        let mut history = EventHistory::with_horizon(0);
        history.insert(event(1, 10));
        history.insert(event(2, 20));
        history.insert(event(3, 30));

        assert_eq!(history.prune_before(&20), PruneResult { removed_ordered: 1, removed_late: 0 });
        assert_eq!(history.iter().map(|(key, _)| key.time).collect::<Vec<_>>(), vec![20, 30]);
    }

    #[test]
    fn pruning_forgets_removed_ids() {
        let mut history = EventHistory::with_horizon(0);
        history.insert(event(1, 10));
        history.prune_before(&20);

        assert_eq!(history.insert(event(1, 20)), Insert::Inserted);
    }

    #[test]
    fn an_older_prune_request_is_a_no_op() {
        let mut history = EventHistory::with_horizon(0);
        history.insert(event(1, 10));
        history.prune_before(&20);

        assert_eq!(history.prune_before(&15), PruneResult { removed_ordered: 0, removed_late: 0 });
        assert_eq!(history.horizon(), &20);
    }

    #[test]
    fn pruning_every_event_moves_the_empty_dirty_boundary_to_the_horizon() {
        let mut history = EventHistory::with_horizon(0);
        history.insert(event(1, 10));
        history.mark_replayed();

        history.prune_before(&20);

        assert!(history.is_empty());
        assert_eq!(history.dirty_time(), &20);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_advance() {
        let mut criterion = Criterion::default();
        criterion.bench_function("events/advance/1000_ordered", |bencher| {
            bencher.iter_batched(
                || {
                    let mut history = EventHistory::with_horizon(0);
                    for value in 0..1_000 {
                        history.insert(event(value, value as i64));
                    }
                    history
                },
                |mut history| black_box(history.prune_before(black_box(&1_000))),
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
