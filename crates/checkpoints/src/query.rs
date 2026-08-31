use crate::{ApplyEvents, ApplyWrapper, CheckpointStore, EventBatch, Events, Snapshot};

/// Reconstructs one query-local snapshot through `time` without modifying
/// retained checkpoints or acknowledging the event history.
pub fn query_at<H, S, W, E, T>(checkpoints: &CheckpointStore<S>, events: &H, wrapper: &mut W, time: T) -> Option<Box<S>>
where
    H: Events<Time = T, Event = E>,
    S: ApplyEvents<E> + Snapshot<Time = T>,
    W: ApplyWrapper<S, E>,
    T: Clone + Default + Ord,
{
    let base_index = checkpoints.checkpoints.partition_point(|checkpoint| checkpoint.key.time <= time).checked_sub(1);
    let mut working_snapshot = base_index.map(|index| checkpoints.checkpoints[index].snapshot.clone());
    let start_key = base_index.map(|index| checkpoints.checkpoints[index].key.clone());
    let mut history_event_count = base_index.map_or(0, |index| checkpoints.checkpoints[index].history_event_count);
    let mut event_iter = events.iter_after(start_key.as_ref()).peekable();
    let mut bucket = Vec::new();

    while let Some(first_event) = event_iter.next() {
        if first_event.time > &time {
            break;
        }

        if working_snapshot.is_none() {
            working_snapshot = Some(S::create(checkpoints.snapshot_id, first_event.event));
        }

        let bucket_time = first_event.time.clone();
        bucket.clear();
        bucket.push(first_event.event);
        while event_iter.peek().is_some_and(|candidate| candidate.time == &bucket_time) {
            bucket.push(event_iter.next().expect("peeked event must exist").event);
        }

        history_event_count = history_event_count
            .checked_add(u64::try_from(bucket.len()).expect("event bucket length exceeded u64"))
            .expect("history event count overflow");
        crate::apply(
            working_snapshot.as_mut().expect("query snapshot must be initialized"),
            EventBatch { snapshot_id: checkpoints.snapshot_id, time: bucket_time, events: &bucket },
            history_event_count,
            wrapper,
        );
    }

    working_snapshot.map(Box::new)
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::Criterion;

    use crate::{query_at, ApplyBatch, ApplyEvents, CheckpointConfig, CheckpointKey, CheckpointStore, EventRef, Events, Snapshot};

    #[derive(Clone)]
    struct TestEvent {
        id: u128,
        time: i64,
        value: i64,
    }

    struct TestEvents {
        dirty_time: i64,
        events: Vec<TestEvent>,
        replay_acknowledgements: usize,
    }

    struct TestEventIter<'a> {
        events: std::slice::Iter<'a, TestEvent>,
    }

    impl<'a> Iterator for TestEventIter<'a> {
        type Item = EventRef<'a, i64, TestEvent>;

        fn next(&mut self) -> Option<Self::Item> {
            self.events.next().map(|event| EventRef { time: &event.time, event_id: event.id, event })
        }
    }

    impl TestEvents {
        fn new(mut events: Vec<TestEvent>) -> Self {
            events.sort_unstable_by_key(|event| (event.time, event.id));
            Self { dirty_time: 0, events, replay_acknowledgements: 0 }
        }
    }

    impl Events for TestEvents {
        type Time = i64;
        type Event = TestEvent;
        type Iter<'a> = TestEventIter<'a>;

        fn dirty_time(&self) -> &Self::Time {
            &self.dirty_time
        }

        fn iter_after(&self, boundary: Option<&CheckpointKey<Self::Time>>) -> Self::Iter<'_> {
            let start = boundary
                .map_or(0, |boundary| self.events.partition_point(|event| (event.time, event.id) <= (boundary.time, boundary.event_id)));
            TestEventIter { events: self.events[start..].iter() }
        }

        fn acknowledge_replay(&mut self) {
            self.replay_acknowledgements += 1;
        }
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    struct TestSnapshot {
        time: i64,
        sum: i64,
        batch_sizes: Vec<usize>,
    }

    impl Snapshot for TestSnapshot {
        type Time = i64;

        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    impl ApplyEvents<TestEvent> for TestSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestEvent) -> Self {
            Self::default()
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, TestEvent>) {
            self.sum += batch.events.iter().map(|event| event.value).sum::<i64>();
            self.batch_sizes.push(batch.events.len());
        }
    }

    fn event(id: u128, time: i64, value: i64) -> TestEvent {
        TestEvent { id, time, value }
    }

    fn replay_fixture(events: &mut TestEvents, interval: u64) -> CheckpointStore<TestSnapshot> {
        let mut store = CheckpointStore::new(7, CheckpointConfig { interval });
        crate::replay(&mut store, events, &mut ());
        store
    }

    #[test]
    fn query_clones_an_exact_checkpoint_without_mutating_retained_state() {
        let mut events = TestEvents::new(vec![event(1, 10, 1), event(2, 20, 2)]);
        let store = replay_fixture(&mut events, 1);
        let before = store.iter().map(|checkpoint| (checkpoint.key.clone(), checkpoint.snapshot.clone())).collect::<Vec<_>>();

        let result = query_at(&store, &events, &mut (), 20).unwrap();

        assert_eq!(*result, TestSnapshot { time: 20, sum: 3, batch_sizes: vec![1, 1] });
        assert_eq!(store.iter().map(|checkpoint| (checkpoint.key.clone(), checkpoint.snapshot.clone())).collect::<Vec<_>>(), before);
        assert_eq!(events.replay_acknowledgements, 1);
    }

    #[test]
    fn query_replays_complete_buckets_after_the_nearest_checkpoint() {
        let mut retained_events = TestEvents::new(vec![event(1, 10, 1), event(2, 20, 2)]);
        let store = replay_fixture(&mut retained_events, 1);
        let query_events = TestEvents::new(vec![event(1, 10, 1), event(2, 20, 2), event(3, 30, 3), event(4, 30, 4), event(5, 40, 5)]);

        let result = query_at(&store, &query_events, &mut (), 30).unwrap();

        assert_eq!(result.time, 30);
        assert_eq!(result.sum, 10);
        assert_eq!(result.batch_sizes, vec![1, 1, 2]);
    }

    #[test]
    fn query_initializes_without_a_checkpoint_and_returns_none_without_events() {
        let store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });
        let events = TestEvents::new(vec![event(1, 10, 1), event(2, 10, 2), event(3, 20, 4)]);

        let result = query_at(&store, &events, &mut (), 10).unwrap();

        assert_eq!(*result, TestSnapshot { time: 10, sum: 3, batch_sizes: vec![2] });
        assert!(query_at(&store, &events, &mut (), 9).is_none());
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_query() {
        let events = TestEvents::new((0..1_000).map(|index| event(index as u128, index as i64, 1)).collect());
        let store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });
        let mut criterion = Criterion::default();

        criterion.bench_function("checkpoints/query/replay_1000_events", |bencher| {
            bencher.iter(|| black_box(query_at(black_box(&store), black_box(&events), &mut (), black_box(999))));
        });
        criterion.final_summary();
    }
}
