use crate::{AdvanceResult, ApplyEvents, ApplyWrapper, CheckpointKey, CheckpointStore, EventBatch, Events, ReplayAnchor, Snapshot};

/// Materializes state through the final event strictly before `horizon` and
/// removes older cadence checkpoints.
pub fn advance_before<H, S, W, E, T>(store: &mut CheckpointStore<S>, events: &H, wrapper: &mut W, horizon: &T) -> AdvanceResult
where
    H: Events<Time = T, Event = E>,
    S: ApplyEvents<E> + Snapshot<Time = T>,
    W: ApplyWrapper<S, E>,
    T: Clone + Default + Ord,
{
    let checkpoint_index = store.checkpoints.partition_point(|checkpoint| checkpoint.key.time < *horizon).checked_sub(1);
    let (mut snapshot, mut boundary, mut history_event_count) = checkpoint_index.map_or_else(
        || {
            store
                .anchor
                .as_ref()
                .map_or((None, None, 0), |anchor| (Some(anchor.snapshot.clone()), anchor.boundary.clone(), anchor.history_event_count))
        },
        |index| {
            let checkpoint = &store.checkpoints[index];
            (Some(checkpoint.snapshot.clone()), Some(checkpoint.key.clone()), checkpoint.history_event_count)
        },
    );

    let mut event_iter = events.iter_after(boundary.as_ref()).peekable();
    let mut bucket = Vec::new();
    while event_iter.peek().is_some_and(|event| event.time < horizon) {
        let first = event_iter.next().expect("peeked pre-horizon event exists");
        if snapshot.is_none() {
            let initial = S::create(store.snapshot_id, first.event);
            store.anchor = Some(ReplayAnchor { boundary: None, snapshot: initial.clone(), history_event_count: 0 });
            snapshot = Some(initial);
        }

        let bucket_time = first.time.clone();
        let mut bucket_last_key = CheckpointKey { time: bucket_time.clone(), event_id: first.event_id };
        bucket.clear();
        bucket.push(first.event);
        while event_iter.peek().is_some_and(|candidate| candidate.time == &bucket_time) {
            let candidate = event_iter.next().expect("peeked same-time event exists");
            bucket_last_key = CheckpointKey { time: bucket_time.clone(), event_id: candidate.event_id };
            bucket.push(candidate.event);
        }

        history_event_count = history_event_count
            .checked_add(u64::try_from(bucket.len()).expect("event bucket length exceeded u64"))
            .expect("history event count overflow");
        crate::apply(
            snapshot.as_mut().expect("pre-horizon event initialized the snapshot"),
            EventBatch { snapshot_id: store.snapshot_id, time: bucket_time, events: &bucket },
            history_event_count,
            wrapper,
        );
        boundary = Some(bucket_last_key);
    }

    if let Some(snapshot) = snapshot {
        store.anchor = Some(ReplayAnchor { boundary, snapshot, history_event_count });
    }

    let removed_checkpoints = store.checkpoints.partition_point(|checkpoint| checkpoint.key.time < *horizon);
    store.checkpoints.drain(..removed_checkpoints);
    AdvanceResult { removed_checkpoints }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::{BatchSize, Criterion};

    use crate::{
        advance_before, replay, ApplyBatch, ApplyEvents, CheckpointConfig, CheckpointKey, CheckpointStore, EventRef, Events, Snapshot,
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
            self.dirty_time = self.events.last().map_or(0, |event| event.time);
        }
    }

    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: i64,
        sum: i64,
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
        }
    }

    fn events(count: u128) -> TestEvents {
        TestEvents { dirty_time: 0, events: (1..=count).map(|value| TestEvent { id: value, time: value as i64, value: 1 }).collect() }
    }

    #[test]
    fn advance_folds_events_after_the_previous_checkpoint_into_the_anchor() {
        let mut events = events(100);
        let mut store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 50 });
        replay(&mut store, &mut events, &mut ());

        advance_before(&mut store, &events, &mut (), &75);

        let anchor = store.anchor().unwrap();
        assert_eq!(anchor.boundary, Some(CheckpointKey { time: 74, event_id: 74 }));
        assert_eq!(anchor.snapshot.sum, 74);
        assert_eq!(anchor.history_event_count, 74);
        assert_eq!(store.iter().map(|checkpoint| checkpoint.key.time).collect::<Vec<_>>(), vec![100]);
    }

    #[test]
    fn replay_after_pruning_starts_from_the_retained_anchor() {
        let mut events = events(100);
        let mut store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 50 });
        replay(&mut store, &mut events, &mut ());
        advance_before(&mut store, &events, &mut (), &75);

        events.events.retain(|event| event.time >= 75);
        events.events.push(TestEvent { id: 101, time: 101, value: 1 });
        events.dirty_time = 80;
        replay(&mut store, &mut events, &mut ());

        assert_eq!(store.current().unwrap().snapshot.sum, 101);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_advance() {
        let mut criterion = Criterion::default();
        criterion.bench_function("checkpoints/advance/1000_events/mid_interval", |bencher| {
            bencher.iter_batched(
                || {
                    let mut events = events(1_000);
                    let mut store = CheckpointStore::<TestSnapshot>::new(7, CheckpointConfig { interval: 100 });
                    replay(&mut store, &mut events, &mut ());
                    (store, events)
                },
                |(mut store, events)| black_box(advance_before(&mut store, &events, &mut (), black_box(&550))),
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
