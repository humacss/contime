use crate::{Apply, Checkpoint, Event, EventStore, Snapshot, Store, Timestamp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardError {
    NoCheckpoint,
    NoPreviousTimestamp,
}
impl std::fmt::Display for ForwardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoCheckpoint => "no valid checkpoint before the horizon",
            Self::NoPreviousTimestamp => "the horizon has no preceding timestamp",
        })
    }
}
impl std::error::Error for ForwardError {}

/// Retains state immediately before the horizon without deleting history.
/// The consumer-supplied forwarding hook runs after each consumed timestamp
/// and at the final predecessor, allowing snapshot-specific compaction.
/// It must preserve observable state and must not advance beyond that time.
pub fn forward<S, H, C>(
    store: &mut Store<S, H>,
    context: &C,
    horizon: S::Time,
    mut hook: impl FnMut(&mut S, &S::Time, &C),
) -> Result<(), ForwardError>
where
    S: Snapshot,
    S::Time: Timestamp,
    H: EventStore<Time = S::Time>,
    H::Event: Apply<Checkpoint<S>, C>,
{
    let before = horizon.previous().ok_or(ForwardError::NoPreviousTimestamp)?;
    let mut playback = store.play(&before).map_err(|_| ForwardError::NoCheckpoint)?;
    let mut last_hook_time = None;
    loop {
        {
            let (checkpoint, events) = playback.begin();
            let mut events = events.take_while(|event| event.time() <= before).peekable();
            if events.peek().is_none() {
                break;
            }
            while let Some(next) = events.peek().map(|event| event.time()) {
                let batch = std::iter::from_fn(|| events.next_if(|event| event.time() == next));
                crate::apply::apply(checkpoint, batch, context);
                hook(&mut checkpoint.snapshot, &next, context);
                last_hook_time = Some(next);
            }
        }
        playback.commit(|commit| commit.forward_checkpoint());
    }
    if last_hook_time.as_ref() != Some(&before) {
        hook(&mut playback.checkpoint.snapshot, &before, context);
    }
    playback.checkpoint.snapshot.set_time(before.clone());
    playback.commit(|commit| {
        commit.forward_checkpoint();
        commit.set_horizon(horizon);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::testing::{TestEvent, TestEventStore, TestSnapshot, Time};
    use crate::Checkpoint;
    use std::hint::black_box;

    #[rstest::rstest]
    #[case::predecessor_has_events(21, true, &[20])]
    #[case::gap_before_horizon(30, true, &[29])]
    #[case::unprocessed_events(30, false, &[10, 20, 29])]
    fn happy(#[case] horizon: Time, #[case] prepared: bool, #[case] expected_hooks: &[Time]) {
        let expected_time = horizon - 1;
        let expected_sum = 6;
        let expected_event_count = 3;
        let expected_checkpoint_count = if prepared { 2 } else { 1 };
        let expected_dirty = if prepared { 20 } else { 0 };
        let expected_events = vec![TestEvent(10, 1), TestEvent(20, 2), TestEvent(20, 3), TestEvent(30, 4)];

        let mut store = Store::new(TestEventStore(expected_events.clone()), TestSnapshot { time: 0, sum: 0 }, 1);
        if prepared {
            store.checkpoints.push_back(Checkpoint { snapshot: TestSnapshot { time: 20, sum: expected_sum }, history_event_count: 3 });
        }
        store.dirty = expected_dirty;
        let mut hooks = Vec::new();

        forward(&mut store, &(), horizon, |_, time, _| hooks.push(*time)).unwrap();
        let actual = &store.checkpoints.back().unwrap();

        assert_eq!(actual.snapshot, TestSnapshot { time: expected_time, sum: expected_sum });
        assert_eq!(actual.history_event_count, expected_event_count);
        assert_eq!(hooks, expected_hooks);
        assert_eq!(store.horizon, horizon);
        assert_eq!(store.dirty, expected_dirty);
        assert_eq!(store.events.0, expected_events);
        assert_eq!(store.checkpoints.len(), expected_checkpoint_count);
    }

    #[test]
    fn empty_history_forwards_across_a_gap() {
        let expected_time: Time = 99;
        let expected_sum = 7;
        let expected_hooks = [expected_time];

        let horizon = expected_time + 1;
        let mut store = Store::new(TestEventStore::default(), TestSnapshot { time: 0, sum: expected_sum }, 100);
        let mut hooks = Vec::new();

        forward(&mut store, &(), horizon, |_, time, _| hooks.push(*time)).unwrap();
        let actual = &store.checkpoints[0].snapshot;

        assert_eq!(actual, &TestSnapshot { time: expected_time, sum: expected_sum });
        assert_eq!(hooks, expected_hooks);
        assert_eq!(store.horizon, horizon);
    }

    #[rstest::rstest]
    #[case::minimum(0, ForwardError::NoPreviousTimestamp)]
    #[case::earlier(5, ForwardError::NoCheckpoint)]
    #[case::repeated(10, ForwardError::NoCheckpoint)]
    fn invalid_target_is_rejected(#[case] requested: Time, #[case] expected_error: ForwardError) {
        let expected_horizon: Time = 10;
        let expected_snapshot = TestSnapshot { time: expected_horizon, sum: 7 };

        let mut store = Store::new(TestEventStore::default(), expected_snapshot.clone(), 100);

        let actual = forward(&mut store, &(), requested, |_, _, _| panic!("unexpected forwarding hook"));

        assert_eq!(actual, Err(expected_error));
        assert_eq!(store.checkpoints[0].snapshot, expected_snapshot);
        assert_eq!(store.horizon, expected_horizon);
        assert_eq!(store.dirty, expected_horizon);
    }

    #[rstest::rstest]
    #[case::missing_checkpoint(true, ForwardError::NoCheckpoint)]
    fn rejected_forward_leaves_storage_unchanged(#[case] empty: bool, #[case] expected_error: ForwardError) {
        let expected_boundary: Time = 0;
        let expected_events = vec![TestEvent(10, 3)];
        let expected_len = usize::from(!empty);

        let horizon: Time = 20;
        let mut store = Store::new(TestEventStore(expected_events.clone()), TestSnapshot { time: 0, sum: 0 }, 100);
        if empty {
            store.checkpoints.clear();
        }

        let actual = forward(&mut store, &(), horizon, |_, _, _| panic!("unexpected forwarding hook"));

        assert_eq!(actual, Err(expected_error));
        assert_eq!(store.horizon, expected_boundary);
        assert_eq!(store.dirty, expected_boundary);
        assert_eq!(store.checkpoints.len(), expected_len);
        assert_eq!(store.events.0, expected_events);
    }

    #[rstest::rstest]
    #[case::partial_intervals(2)]
    #[case::one_interval_crosses_several_slots(100)]
    fn stale_checkpoints_are_reused_in_order(#[case] interval: u64) {
        let expected_time: Time = 28;
        let expected_sum = 7;
        let expected_dirty = 0;
        let expected_checkpoint_count = 4;

        let horizon = expected_time + 1;
        let events = TestEventStore(vec![TestEvent(5, 1), TestEvent(15, 2), TestEvent(25, 4), TestEvent(30, 8)]);
        let mut store = Store::new(events, TestSnapshot { time: 0, sum: 0 }, interval);
        store
            .checkpoints
            .extend([10, 20, 30].map(|time| Checkpoint { snapshot: TestSnapshot { time, sum: 999 }, history_event_count: time / 10 }));

        forward(&mut store, &(), horizon, |_, _, _| {}).unwrap();
        let actual_times = store.checkpoints.iter().map(|checkpoint| checkpoint.snapshot.time).collect::<Vec<_>>();
        let actual = store.play(&horizon).unwrap().checkpoint.snapshot;

        assert_eq!(actual, TestSnapshot { time: expected_time, sum: expected_sum });
        assert!(actual_times.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(store.checkpoints.len(), expected_checkpoint_count);
        assert_eq!(store.dirty, expected_dirty);
        assert_eq!(store.horizon, horizon);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_forward_unit() {
        let event_count = 1000;
        let interval = 100;
        let horizon = event_count + 1;
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        criterion.bench_function("checkpoints/forward/1000_events_10_intervals", |b| {
            b.iter_batched_ref(
                || {
                    let events = TestEventStore((1..=event_count).map(|time| TestEvent(time, 1)).collect());
                    Store::new(events, TestSnapshot { time: 0, sum: 0 }, interval)
                },
                |store| {
                    forward(black_box(store), black_box(&()), black_box(horizon), |snapshot, time, _| {
                        black_box((snapshot, time));
                    })
                    .unwrap();
                    black_box((&store.checkpoints, store.horizon));
                },
                criterion::BatchSize::SmallInput,
            );
        });
        criterion.final_summary();
    }
}
