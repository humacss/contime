use crate::types::{History, HistoryIter};
use crate::{Input, SharedEvent};

impl<I: Input> History<I> {
    pub(crate) fn with_horizon(horizon: I::Time) -> Self {
        Self { events: contime_events::EventHistory::with_horizon(horizon) }
    }
}

impl<'a, I> Iterator for HistoryIter<'a, I>
where
    I: Input,
{
    type Item = &'a SharedEvent<I>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = match self {
            Self::All(iter) => iter.next(),
            Self::Range(iter) => iter.next(),
        }?;
        Some(next.1)
    }
}

impl<I> contime_checkpoints::EventStore for History<I>
where
    I: Input,
{
    type Time = I::Time;
    type Event = SharedEvent<I>;
    type Iter<'a>
        = HistoryIter<'a, I>
    where
        Self: 'a;

    fn iter_after(&self, boundary: Option<&Self::Time>) -> Self::Iter<'_> {
        match boundary {
            Some(boundary) => {
                HistoryIter::Range(self.events.iter_after(&contime_events::EventKey { time: boundary.clone(), event_id: u128::MAX }))
            }
            None => HistoryIter::All(self.events.iter()),
        }
    }

    fn prune_before(&mut self, horizon: &Self::Time) {
        self.events.prune_before(horizon);
    }
}

impl<I: Input> contime_checkpoints::InsertEventStore for History<I> {
    fn insert(&mut self, event: SharedEvent<I>) -> contime_checkpoints::Insert {
        match self.events.insert(event) {
            contime_events::Insert::Inserted => contime_checkpoints::Insert::Inserted,
            contime_events::Insert::Duplicate => contime_checkpoints::Insert::Duplicate,
            contime_events::Insert::BeforeHorizon => contime_checkpoints::Insert::BeforeHorizon,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::types::testing::Time;
    use std::hint::black_box;

    use contime_checkpoints::EventStore as ReplayEvents;

    use contime_checkpoints::{Insert, InsertEventStore};
    use criterion::{BatchSize, Criterion};

    use crate::input::prepare_inputs;
    use crate::types::History;
    use crate::Input;

    #[derive(Debug)]
    struct TestInput {
        id: u128,
        time: Time,
    }

    impl contime_checkpoints::Event for TestInput {
        type Time = Time;

        fn time(&self) -> Self::Time {
            self.time
        }
    }
    impl Input for TestInput {
        fn event_id(&self) -> u128 {
            self.id
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }

    fn event(id: u128, time: Time) -> crate::SharedEvent<TestInput> {
        prepare_inputs(vec![TestInput { id, time }]).pop().unwrap()
    }

    #[test]
    fn history_deduplicates_and_exposes_raw_events_in_canonical_order() {
        let mut history = History::<TestInput>::with_horizon(Time(0));

        assert_eq!(history.insert(event(2, Time(20))), Insert::Inserted);
        assert_eq!(history.insert(event(1, Time(10))), Insert::Inserted);
        assert_eq!(history.insert(event(1, Time(30))), Insert::Duplicate);

        let retained = ReplayEvents::iter_after(&history, None).map(|event| (event.id, event.time, event.id)).collect::<Vec<_>>();
        assert_eq!(retained, vec![(1, Time(10), 1), (2, Time(20), 2)]);
        assert_eq!(history.events.dirty_time(), &Time(10));
    }

    #[test]
    fn pre_horizon_inputs_are_rejected() {
        let mut history = History::<TestInput>::with_horizon(Time(10));

        let rejection = history.insert(event(1, Time(9)));

        assert_eq!(rejection, Insert::BeforeHorizon);

        assert_eq!(history.insert(event(2, Time(10))), Insert::Inserted);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_history() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/history/1000_ordered_inserts", |bencher| {
            bencher.iter_batched(
                || {
                    let inputs = prepare_inputs((0..1_000).map(|id| TestInput { id, time: Time(id as i64) }).collect());
                    (History::with_horizon(Time(0)), inputs)
                },
                |(mut history, inputs)| {
                    for input in inputs {
                        black_box(history.insert(input));
                    }
                    black_box(history)
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
