use crate::{ApplyEvents, ApplyResult, ApplyWrapper, CheckpointKey, CheckpointStore, EventBatch, EventRef, Events};

/// Iterates changed canonical event buckets, commits checkpoint state, and
/// acknowledges the event history after successful completion.
pub fn replay<H, S, W, E, T>(checkpoints: &mut CheckpointStore<S>, events: &mut H, wrapper: &mut W) -> ApplyResult
where
    H: Events<Time = T, Event = E>,
    S: ApplyEvents<E> + crate::Snapshot<Time = T>,
    W: ApplyWrapper<S, E>,
    T: Clone + Default + Ord,
{
    let result = replay_events(checkpoints, events, wrapper);
    events.acknowledge_replay();
    result
}

fn replay_events<H, S, W, E, T>(checkpoints: &mut CheckpointStore<S>, events: &H, wrapper: &mut W) -> ApplyResult
where
    H: Events<Time = T, Event = E>,
    S: ApplyEvents<E> + crate::Snapshot<Time = T>,
    W: ApplyWrapper<S, E>,
    T: Clone + Default + Ord,
{
    let snapshot_id = checkpoints.snapshot_id();
    let mut session = checkpoints.begin_replay(events.dirty_time());
    let start_key = session.start_key().cloned();
    let mut event_iter = events.iter_after(start_key.as_ref()).peekable();
    let Some(first_event) = event_iter.next() else {
        return session.unchanged();
    };

    if session.working_snapshot.is_none() {
        session.initialize(S::create(snapshot_id, first_event.event));
    }

    let mut bucket = Vec::new();
    let mut next_event = Some(first_event);

    while let Some(event) = next_event.take() {
        let bucket_time = event.time.clone();
        let mut bucket_last_key = key_from_event(&event);
        bucket.clear();
        bucket.push(event.event);

        while event_iter.peek().is_some_and(|candidate| candidate.time == &bucket_time) {
            let candidate = event_iter.next().expect("peeked event must exist");
            bucket_last_key = key_from_event(&candidate);
            bucket.push(candidate.event);
        }

        let bucket_count = u64::try_from(bucket.len()).expect("event bucket length exceeded u64");
        let history_event_count = session.advance_event_count(bucket_count);
        crate::apply(session.snapshot_mut(), EventBatch { snapshot_id, time: bucket_time, events: &bucket }, history_event_count, wrapper);

        next_event = event_iter.next();
        if next_event.is_some() {
            session.record_intermediate(bucket_last_key);
        } else {
            return session.finish(bucket_last_key);
        }
    }

    unreachable!("replay returns after its final event bucket")
}

fn key_from_event<T, E>(event: &EventRef<'_, T, E>) -> CheckpointKey<T>
where
    T: Clone,
{
    CheckpointKey { time: event.time.clone(), event_id: event.event_id }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::{BatchSize, Criterion};

    use super::replay;
    use crate::{
        ApplyBatch, ApplyEvents, ApplyWrapper, CheckpointConfig, CheckpointKey, CheckpointStore, EventBatch, EventRef, Events, Snapshot,
    };

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
        fn new(dirty_time: i64, mut events: Vec<TestEvent>) -> Self {
            events.sort_unstable_by_key(|event| (event.time, event.id));
            Self { dirty_time, events, replay_acknowledgements: 0 }
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

    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: i64,
        sum: i64,
        batch_sizes: Vec<usize>,
        batch_times: Vec<i64>,
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
            self.batch_times.push(batch.time);
        }
    }

    #[derive(Clone, Default)]
    struct BenchmarkSnapshot {
        time: i64,
        sum: i64,
    }

    impl Snapshot for BenchmarkSnapshot {
        type Time = i64;

        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    impl ApplyEvents<TestEvent> for BenchmarkSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestEvent) -> Self {
            Self::default()
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, TestEvent>) {
            self.sum += batch.events.iter().map(|event| event.value).sum::<i64>();
        }
    }

    fn event(id: u128, time: i64, value: i64) -> TestEvent {
        TestEvent { id, time, value }
    }

    #[test]
    fn replay_applies_every_complete_timestamp_bucket_once() {
        let mut events = TestEvents::new(0, vec![event(2, 10, 2), event(1, 10, 1), event(3, 20, 3)]);
        let mut checkpoints = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });

        let result = replay(&mut checkpoints, &mut events, &mut ());

        let snapshot = &checkpoints.current().unwrap().snapshot;
        assert_eq!(snapshot.sum, 6);
        assert_eq!(snapshot.batch_sizes, vec![2, 1]);
        assert_eq!(snapshot.batch_times, vec![10, 20]);
        assert_eq!(result.applied_events, 3);
    }

    #[test]
    fn successful_replay_acknowledges_the_canonical_event_history_once() {
        let mut events = TestEvents::new(0, vec![event(1, 10, 1)]);
        let mut checkpoints = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });

        replay(&mut checkpoints, &mut events, &mut ());

        assert_eq!(events.replay_acknowledgements, 1);
    }

    #[test]
    fn unchanged_replay_still_acknowledges_the_canonical_event_history() {
        let mut events = TestEvents::new(0, Vec::new());
        let mut checkpoints = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });

        let result = replay(&mut checkpoints, &mut events, &mut ());

        assert_eq!(result.applied_events, 0);
        assert_eq!(events.replay_acknowledgements, 1);
    }

    struct PanicWrapper;

    impl ApplyWrapper<TestSnapshot, TestEvent> for PanicWrapper {
        fn apply_event_batch(&mut self, _batch: EventBatch<'_, i64, TestEvent>, _apply_inner: &mut crate::ApplyInner<'_, TestSnapshot>) {
            panic!("replay failed");
        }
    }

    #[test]
    fn failed_replay_does_not_acknowledge_the_canonical_event_history() {
        let mut events = TestEvents::new(0, vec![event(1, 10, 1)]);
        let mut checkpoints = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });

        let replay_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            replay(&mut checkpoints, &mut events, &mut PanicWrapper);
        }));

        assert!(replay_result.is_err());
        assert_eq!(events.replay_acknowledgements, 0);
    }

    #[test]
    fn late_replay_updates_existing_checkpoints_and_only_appends_at_the_tip() {
        let mut initial = TestEvents::new(0, vec![event(1, 10, 1), event(2, 20, 2), event(3, 30, 3), event(4, 40, 4)]);
        let mut checkpoints = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 2 });
        replay(&mut checkpoints, &mut initial, &mut ());
        assert_eq!(checkpoints.len(), 2);

        let mut corrected = TestEvents::new(15, vec![event(1, 10, 1), event(5, 15, 5), event(2, 20, 2), event(3, 30, 3), event(4, 40, 4)]);
        replay(&mut checkpoints, &mut corrected, &mut ());

        assert_eq!(checkpoints.len(), 3);
        assert_eq!(
            checkpoints.iter().map(|checkpoint| checkpoint.key.clone()).collect::<Vec<_>>(),
            vec![CheckpointKey { time: 15, event_id: 5 }, CheckpointKey { time: 30, event_id: 3 }, CheckpointKey { time: 40, event_id: 4 },]
        );
        assert_eq!(checkpoints.current().unwrap().snapshot.sum, 15);
    }

    fn benchmark_events(one_timestamp: bool) -> TestEvents {
        TestEvents::new(0, (0..1_000).map(|index| event(index as u128, if one_timestamp { 10 } else { index as i64 }, 1)).collect())
    }

    fn benchmark_replay_case(criterion: &mut Criterion, name: &str, events: &mut TestEvents) {
        criterion.bench_function(name, |bencher| {
            bencher.iter_batched(
                || CheckpointStore::<BenchmarkSnapshot>::new(7, CheckpointConfig { interval: 100 }),
                |mut checkpoints| {
                    let result = replay(&mut checkpoints, events, &mut ());
                    black_box((checkpoints, result))
                },
                BatchSize::LargeInput,
            );
        });
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_replay() {
        let mut one_timestamp = benchmark_events(true);
        let mut unique_timestamps = benchmark_events(false);
        let mut criterion = Criterion::default();

        benchmark_replay_case(&mut criterion, "checkpoints/replay/1000_events/one_timestamp", &mut one_timestamp);
        benchmark_replay_case(&mut criterion, "checkpoints/replay/1000_events/unique_timestamps", &mut unique_timestamps);
        criterion.final_summary();
    }
}
