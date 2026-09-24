use crate::{Apply, Checkpoint, Event, EventStore, NoCheckpoint, Playback, Snapshot, Store};

/// Rebuilds and retains the dirty suffix through the inclusive target.
pub fn replay<S, H, C>(store: &mut Store<S, H>, context: &C, time: S::Time) -> Result<(), NoCheckpoint>
where
    S: Snapshot,
    H: EventStore<Time = S::Time>,
    H::Event: Apply<Checkpoint<S>, C>,
{
    let mut playback = store.play(&time)?;
    loop {
        {
            let (checkpoint, events) = playback.begin();
            let mut events = events.take_while(|event| event.time() <= time).peekable();
            if events.peek().is_none() {
                break;
            }
            crate::apply::apply(checkpoint, events, context);
        }
        retain(&mut playback);
    }
    Ok(())
}

// Replay chooses placement; Commit only performs the requested writes.
fn retain<S: Snapshot, H: EventStore<Time = S::Time>>(playback: &mut Playback<'_, S, H>) {
    let index = playback.checkpoint_index;
    let checkpoints = &playback.store.checkpoints;
    let replace_current = index > 0
        && (playback.store.checkpoint_interval == 0
            || checkpoints[index].history_event_count.saturating_sub(checkpoints[index - 1].history_event_count)
                < playback.store.checkpoint_interval);
    let next = if replace_current { index } else { index + 1 };
    let time = playback.checkpoint.snapshot.time();
    let last = checkpoints.partition_point(|checkpoint| checkpoint.snapshot.time() <= time).saturating_sub(1).max(next);
    let append = next == checkpoints.len();
    let dirty = playback.store.dirty.clone().max(time.clone());
    playback.commit(|commit| {
        if append {
            commit.push_checkpoint();
        } else {
            // Do not expose an overtaken stale checkpoint as valid. Reuse its
            // slot; physical deduplication can happen during later cleanup.
            for index in next..=last {
                commit.replace_checkpoint(index);
            }
        }
        commit.set_dirty(dirty);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    use crate::types::testing::{TestEvent, TestEventStore, TestSnapshot, Time};

    #[rstest::rstest]
    #[case::bounded(2, &[0, 20, 30])]
    #[case::unbounded(0, &[0, 30])]
    fn happy(#[case] interval: u64, #[case] expected_times: &[Time]) {
        let expected_sum = 10;
        let expected_time: Time = 30;
        let expected_count = 4;

        let events = TestEventStore(vec![TestEvent(10, 1), TestEvent(20, 2), TestEvent(20, 3), TestEvent(30, 4)]);
        let mut store = Store::new(events, TestSnapshot { time: 0, sum: 0 }, interval);

        replay(&mut store, &(), expected_time).unwrap();
        let actual_times = store.checkpoints.iter().map(|checkpoint| checkpoint.snapshot.time).collect::<Vec<_>>();
        let actual_tip = store.checkpoints.back().unwrap();

        assert_eq!(actual_times, expected_times);
        assert_eq!(actual_tip.snapshot.sum, expected_sum);
        assert_eq!(actual_tip.history_event_count, expected_count);
        assert_eq!(store.dirty, expected_time);
    }

    #[test]
    fn partial_interval_reuses_the_tip() {
        let expected_times = [0, 20];
        let expected_sum = 3;

        let events = TestEventStore(vec![TestEvent(10, 1), TestEvent(20, 2), TestEvent(30, 4)]);
        let mut store = Store::new(events, TestSnapshot { time: 0, sum: 0 }, 3);

        replay(&mut store, &(), 10).unwrap();
        replay(&mut store, &(), 20).unwrap();
        let actual_times = store.checkpoints.iter().map(|checkpoint| checkpoint.snapshot.time).collect::<Vec<_>>();
        let actual_sum = store.checkpoints.back().unwrap().snapshot.sum;

        assert_eq!(actual_times, expected_times);
        assert_eq!(actual_sum, expected_sum);
    }

    #[test]
    fn replay_stops_at_the_inclusive_target() {
        let expected_time: Time = 10;
        let expected_sum = 3;

        let events = TestEventStore(vec![TestEvent(10, 1), TestEvent(10, 2), TestEvent(20, 4)]);
        let mut store = Store::new(events, TestSnapshot { time: 0, sum: 0 }, 1);
        let target = expected_time;

        replay(&mut store, &(), target).unwrap();
        let actual = &store.checkpoints.back().unwrap().snapshot;

        assert_eq!(actual.time, expected_time);
        assert_eq!(actual.sum, expected_sum);
        assert_eq!(store.dirty, expected_time);
    }

    #[test]
    fn dirty_replay_overwrites_stale_state() {
        let expected_sum = 7;
        let expected_len = 4;
        let expected_dirty: Time = 30;

        let events = TestEventStore(vec![TestEvent(10, 1), TestEvent(20, 2), TestEvent(30, 4)]);
        let mut store = Store::new(events, TestSnapshot { time: 0, sum: 0 }, 1);
        store
            .checkpoints
            .extend([10, 20, 30].map(|time| Checkpoint { snapshot: TestSnapshot { time, sum: 999 }, history_event_count: time / 10 }));

        replay(&mut store, &(), expected_dirty).unwrap();
        let actual_sum = store.checkpoints.back().unwrap().snapshot.sum;
        let actual_len = store.checkpoints.len();

        assert_eq!(actual_sum, expected_sum);
        assert_eq!(actual_len, expected_len);
        assert_eq!(store.dirty, expected_dirty);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_replay_unit() {
        let event_count = 1000;
        let interval = 100;
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        for dirty in [false, true] {
            let name = if dirty { "checkpoints/replay/rebuild_1000" } else { "checkpoints/replay/fresh_1000" };
            criterion.bench_function(name, |b| {
                b.iter_batched_ref(
                    || {
                        let events = TestEventStore((1..=event_count).map(|time| TestEvent(time, 1)).collect());
                        let mut store = Store::new(events, TestSnapshot { time: 0, sum: 0 }, interval);
                        if dirty {
                            replay(&mut store, &(), event_count).unwrap();
                            store.dirty = 0;
                        }
                        store
                    },
                    |store| {
                        replay(black_box(store), black_box(&()), black_box(event_count)).unwrap();
                        black_box((&store.checkpoints, store.dirty));
                    },
                    criterion::BatchSize::SmallInput,
                );
            });
        }
        criterion.final_summary();
    }
}
