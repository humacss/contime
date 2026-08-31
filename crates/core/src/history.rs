use std::convert::Infallible;

use contime_checkpoints::{CheckpointKey, EventRef};
use contime_worker::EventInsert;

use crate::types::{History, HistoryIter};
use crate::{Input, TrackedEvent};

impl<I> History<I>
where
    I: Input,
{
    pub(crate) fn new() -> Self {
        Self { events: contime_events::EventHistory::new() }
    }

    pub(crate) fn insert(&mut self, input: TrackedEvent<I>) -> EventInsert<Infallible> {
        let changed = self.events.insert(input).changed();
        EventInsert { changed, rejections: Vec::new() }
    }
}

impl<I> contime_worker::Events<TrackedEvent<I>> for History<I>
where
    I: Input,
{
    type Config = ();
    type Rejection = Infallible;

    fn create(_snapshot_id: u128, _config: &Self::Config) -> Self {
        Self::new()
    }

    fn insert(&mut self, input: TrackedEvent<I>) -> EventInsert<Self::Rejection> {
        self.insert(input)
    }
}

impl<I> contime_worker::QueryEvents<TrackedEvent<I>> for History<I>
where
    I: Input,
{
    type Time = I::Time;

    fn clone_between(&self, from: &Self::Time, to: &Self::Time) -> Vec<TrackedEvent<I>> {
        self.events.clone_between(from, to)
    }
}

impl<'a, I> Iterator for HistoryIter<'a, I>
where
    I: Input,
{
    type Item = EventRef<'a, I::Time, I>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = match self {
            Self::All(iter) => iter.next(),
            Self::Range(iter) => iter.next(),
        }?;
        Some(EventRef { time: &next.0.time, event_id: next.0.event_id, event: next.1.as_ref() })
    }
}

impl<I> contime_checkpoints::Events for History<I>
where
    I: Input,
{
    type Time = I::Time;
    type Event = I;
    type Iter<'a>
        = HistoryIter<'a, I>
    where
        Self: 'a;

    fn dirty_time(&self) -> &Self::Time {
        self.events.dirty_time()
    }

    fn iter_after(&self, boundary: Option<&CheckpointKey<Self::Time>>) -> Self::Iter<'_> {
        match boundary {
            Some(boundary) => HistoryIter::Range(
                self.events.iter_after(&contime_events::EventKey { time: boundary.time.clone(), event_id: boundary.event_id }),
            ),
            None => HistoryIter::All(self.events.iter()),
        }
    }

    fn acknowledge_replay(&mut self) {
        self.events.mark_replayed();
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use contime_checkpoints::Events as ReplayEvents;
    use contime_memory::ConservativeTrackedSize;
    use contime_worker::Events as WorkerEvents;
    use criterion::{BatchSize, Criterion};

    use crate::input::prepare_inputs;
    use crate::types::History;
    use crate::{Input, MemoryBudget};

    #[derive(Debug)]
    struct TestInput {
        id: u128,
        time: i64,
    }

    impl ConservativeTrackedSize for TestInput {
        fn conservative_tracked_size(&self) -> usize {
            32
        }
    }

    impl Input for TestInput {
        type Time = i64;

        fn event_id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> Self::Time {
            self.time
        }

        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }

    fn event(budget: &MemoryBudget, id: u128, time: i64) -> crate::TrackedEvent<TestInput> {
        prepare_inputs(budget, vec![TestInput { id, time }]).unwrap().pop().unwrap()
    }

    #[test]
    fn history_deduplicates_and_exposes_raw_events_in_canonical_order() {
        let budget = MemoryBudget::new(10_000, 100);
        let mut history = <History<TestInput> as WorkerEvents<_>>::create(7, &());

        assert!(history.insert(event(&budget, 2, 20)).changed);
        assert!(history.insert(event(&budget, 1, 10)).changed);
        assert!(!history.insert(event(&budget, 1, 30)).changed);

        let retained = ReplayEvents::iter_after(&history, None)
            .map(|event| (event.event_id, event.time.to_owned(), event.event.id))
            .collect::<Vec<_>>();
        assert_eq!(retained, vec![(1, 10, 1), (2, 20, 2)]);
        assert_eq!(ReplayEvents::dirty_time(&history), &10);
    }

    #[test]
    fn replay_acknowledgement_moves_dirty_time_to_the_latest_event() {
        let budget = MemoryBudget::new(10_000, 100);
        let mut history = <History<TestInput> as WorkerEvents<_>>::create(7, &());
        history.insert(event(&budget, 2, 20));
        history.insert(event(&budget, 1, 10));

        ReplayEvents::acknowledge_replay(&mut history);

        assert_eq!(ReplayEvents::dirty_time(&history), &20);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_history() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/history/1000_ordered_inserts", |bencher| {
            bencher.iter_batched(
                || {
                    let budget = MemoryBudget::new(usize::MAX, 0);
                    let inputs = prepare_inputs(&budget, (0..1_000).map(|id| TestInput { id, time: id as i64 }).collect()).unwrap();
                    (History::new(), inputs)
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
