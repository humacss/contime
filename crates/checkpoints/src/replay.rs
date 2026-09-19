use crate::{ApplyEvents, ApplyResult, ApplyWrapper, CheckpointKey, CheckpointStore, EventBatch, EventRef, Events};

/// Applies at most one complete canonical timestamp bucket through `target`.
///
/// The valid tip is the continuation cursor. After a history mutation, first
/// call `invalidate_from` with its earliest affected timestamp. This function
/// does not acknowledge the history: the caller owns incremental scheduling.
pub fn replay_next<H, S, W, E, T>(checkpoints: &mut CheckpointStore<S>, events: &H, wrapper: &mut W, target: &T) -> ApplyResult
where
    H: Events<Time = T, Event = E>,
    S: ApplyEvents<E> + crate::Snapshot<Time = T>,
    W: ApplyWrapper<S, E>,
    T: Clone + Default + Ord,
{
    let mut iter = events.iter_after(checkpoints.replay_boundary());
    let Some(first) = iter.next().filter(|event| event.time <= target) else {
        return ApplyResult { applied_events: 0, retained_checkpoints: checkpoints.len() };
    };
    let time = first.time.clone();
    let mut last_key = key_from_event(&first);
    let mut bucket = vec![first.event];
    for event in iter {
        if event.time != &time {
            break;
        }
        last_key = key_from_event(&event);
        bucket.push(event.event);
    }
    let snapshot_id = checkpoints.snapshot_id();
    let mut session = checkpoints.resume_replay();
    if session.working_snapshot.is_none() {
        let mut snapshot = S::create(snapshot_id, first.event);
        snapshot.set_time(T::default());
        session.initialize(snapshot);
    }
    let count = session.advance_event_count(u64::try_from(bucket.len()).expect("event bucket length exceeded u64"));
    let mut inner = crate::ApplyInner::new(session.snapshot_mut(), count);
    wrapper.replay_event_batch(EventBatch { snapshot_id, time, events: &bucket }, &mut inner);
    assert!(inner.has_applied(), "a replay wrapper must call the inner apply at least once per event batch");
    session.finish(last_key)
}

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
        let mut snapshot = S::create(snapshot_id, first_event.event);
        snapshot.set_time(T::default());
        session.initialize(snapshot);
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
        let mut inner = crate::ApplyInner::new(session.snapshot_mut(), history_event_count);
        wrapper.replay_event_batch(EventBatch { snapshot_id, time: bucket_time, events: &bucket }, &mut inner);
        assert!(inner.has_applied(), "a replay wrapper must call the inner apply at least once per event batch");

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
            Self { time: 999, ..Self::default() }
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
    fn incremental_replay_yields_after_a_complete_timestamp_and_respects_target() {
        let mut store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 3 });
        let events = TestEvents::new(10, vec![event(2, 10, 2), event(1, 10, 1), event(3, 20, 3), event(4, 30, 4)]);
        assert_eq!(store.next_replay_time(&events), Some(10));
        assert_eq!(super::replay_next(&mut store, &events, &mut (), &20).applied_events, 2);
        assert_eq!(store.current().unwrap().snapshot.batch_sizes, vec![2]);
        assert_eq!(store.next_replay_time(&events), Some(20));
        assert_eq!(super::replay_next(&mut store, &events, &mut (), &20).applied_events, 1);
        assert_eq!(store.current().unwrap().snapshot.sum, 6);
        assert_eq!(store.len(), 1);
        assert_eq!(super::replay_next(&mut store, &events, &mut (), &20).applied_events, 0);
        assert_eq!(store.next_replay_time(&events), Some(30));
        super::replay_next(&mut store, &events, &mut (), &30);
        assert_eq!(store.current().unwrap().snapshot.batch_times, vec![10, 20, 30]);
        assert_eq!(store.len(), 2);
        assert_eq!(store.next_replay_time(&events), None);
        assert_eq!(events.replay_acknowledgements, 0);
    }

    #[test]
    fn incremental_late_insertion_invalidates_queries_and_replays_the_whole_bucket() {
        let mut store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 1 });
        let original = TestEvents::new(10, vec![event(1, 10, 1), event(3, 20, 3), event(4, 30, 4)]);
        while store.next_replay_time(&original).is_some() {
            super::replay_next(&mut store, &original, &mut (), &30);
        }
        let edited = TestEvents::new(20, vec![event(1, 10, 1), event(2, 20, 2), event(3, 20, 3), event(4, 30, 4)]);
        store.invalidate_from(&20);
        assert_eq!(store.current().unwrap().key.time, 10);
        assert_eq!(crate::query_at(&store, &edited, &mut (), 30).unwrap().sum, 10);
        assert_eq!(store.current().unwrap().snapshot.sum, 1);
        assert_eq!(super::replay_next(&mut store, &edited, &mut (), &30).applied_events, 2);
        assert_eq!(store.current().unwrap().snapshot.batch_times, vec![10, 20]);
        assert_eq!(store.current().unwrap().snapshot.sum, 6);
        super::replay_next(&mut store, &edited, &mut (), &30);
        assert_eq!(store.current().unwrap().snapshot.sum, 10);
    }

    #[test]
    fn incremental_replay_matches_full_replay_after_late_edits_and_pruning() {
        for interval in [0, 1, 3, 10, 100] {
            let mut store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval });
            let mut events = TestEvents::new(0, (0..20).map(|id| event(id, id as i64 * 10, 1)).collect());
            while store.next_replay_time(&events).is_some() {
                super::replay_next(&mut store, &events, &mut (), &200);
            }
            crate::advance_before(&mut store, &events, &mut (), &65);
            events.events.retain(|event| event.time >= 65);
            for (id, time) in [(101, 150), (102, 70), (103, 150), (104, 65)] {
                events.events.push(event(id, time, 3));
                events.events.sort_by_key(|event| (event.time, event.id));
                store.invalidate_from(&time);
                while store.next_replay_time(&events).is_some() {
                    super::replay_next(&mut store, &events, &mut (), &200);
                }
                let all = (0..20)
                    .map(|id| event(id, id as i64 * 10, 1))
                    .chain(events.events.iter().filter(|event| event.id >= 100).cloned())
                    .collect();
                let mut reference_events = TestEvents::new(0, all);
                let mut reference = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval });
                replay(&mut reference, &mut reference_events, &mut ());
                for query_time in [65, 70, 100, 150, 200] {
                    let actual = crate::query_at(&store, &events, &mut (), query_time).unwrap();
                    let expected = crate::query_at(&reference, &reference_events, &mut (), query_time).unwrap();
                    assert_eq!(actual.sum, expected.sum, "interval={interval}, edit={time}, query={query_time}");
                    assert_eq!(actual.batch_times, expected.batch_times);
                }
                assert_eq!(store.current().unwrap().history_event_count, reference.current().unwrap().history_event_count);
            }
        }
    }

    #[test]
    fn incremental_replay_moves_the_partial_tip_without_cloning_on_each_step() {
        struct CountClones(std::rc::Rc<std::cell::Cell<usize>>);
        impl Clone for CountClones {
            fn clone(&self) -> Self {
                self.0.set(self.0.get() + 1);
                Self(self.0.clone())
            }
        }
        impl Snapshot for CountClones {
            type Time = i64;
            fn set_time(&mut self, _: i64) {}
        }
        impl ApplyEvents<TestEvent> for CountClones {
            fn create(_: u128, _: &TestEvent) -> Self {
                Self(Default::default())
            }
            fn apply_events(&mut self, _: ApplyBatch<'_, i64, TestEvent>) {}
        }
        let events = TestEvents::new(0, (0..20).map(|id| event(id, id as i64, 1)).collect());
        let mut store = CheckpointStore::<CountClones>::new(7, CheckpointConfig { interval: 100 });
        for _ in 0..20 {
            super::replay_next(&mut store, &events, &mut (), &20);
        }
        // The initial clean anchor is the only clone; every partial tip moves.
        assert_eq!(store.current().unwrap().snapshot.0.get(), 1);
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
    fn first_materialization_retains_the_clean_initial_snapshot() {
        let mut events = TestEvents::new(0, vec![event(1, 10, 5)]);
        let mut checkpoints = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });

        replay(&mut checkpoints, &mut events, &mut ());

        let anchor = checkpoints.anchor().expect("materialized history has an anchor");
        assert_eq!(anchor.boundary, None);
        assert_eq!(anchor.snapshot.time, 0);
        assert_eq!(anchor.snapshot.sum, 0);
        assert_eq!(anchor.history_event_count, 0);
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
        criterion.bench_function("checkpoints/replay_next/1000_events/unique_timestamps", |bencher| {
            bencher.iter_batched(
                || CheckpointStore::<BenchmarkSnapshot>::new(7, CheckpointConfig { interval: 100 }),
                |mut checkpoints| {
                    while super::replay_next(&mut checkpoints, &unique_timestamps, &mut (), &1_000).applied_events != 0 {}
                    black_box(checkpoints)
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
