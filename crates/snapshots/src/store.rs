//! Storage ownership and playback initialization.
use super::{Checkpoint, Playback};
use crate::{EventStore, NoCheckpoint, Snapshot};
use std::collections::VecDeque;

/// Storage for one snapshot. Dirty is the inclusive valid-through boundary.
/// Commit callbacks own checkpoint retention and validity updates.
pub struct Store<S: Snapshot, H> {
    pub(super) events: H,
    pub(super) checkpoints: VecDeque<Checkpoint<S>>,
    /// Zero leaves event intervals unbounded.
    pub(super) checkpoint_interval: u64,
    pub(super) dirty: S::Time,
    /// Completed forwarding boundary; playback cannot begin before it.
    pub(super) horizon: S::Time,
}

impl<S: Snapshot, H> Store<S, H> {
    pub fn new(events: H, snapshot: S, checkpoint_interval: u64) -> Self {
        let dirty = snapshot.time().clone();
        Self {
            events,
            checkpoints: VecDeque::from([Checkpoint { snapshot, history_event_count: 0 }]),
            checkpoint_interval,
            horizon: dirty.clone(),
            dirty,
        }
    }

    /// Rejects requests before the horizon, then starts from a valid checkpoint.
    /// At the horizon, start from its predecessor so events at the horizon remain eligible.
    pub fn play(&mut self, start: &S::Time) -> Result<Playback<'_, S, H>, NoCheckpoint> {
        if self.checkpoints.is_empty() || start < &self.horizon {
            return Err(NoCheckpoint);
        }
        let boundary = start.min(&self.dirty);
        // Forwarded state remains valid even when replay's dirty boundary is older.
        let retained = self.checkpoints.partition_point(|checkpoint| checkpoint.snapshot.time() < &self.horizon);
        if let Some(index) = retained.checked_sub(1) {
            if self.checkpoints[index].snapshot.time() > boundary {
                return Ok(Playback::new(self, index));
            }
        }
        let end = self.checkpoints.partition_point(|checkpoint| {
            checkpoint.snapshot.time() <= boundary && (start != &self.horizon || checkpoint.snapshot.time() < start)
        });
        let index = match end.checked_sub(1) {
            Some(index) => index,
            // The initial snapshot precedes all events, including those at its own time.
            None if self.checkpoints[0].history_event_count == 0 && self.checkpoints[0].snapshot.time() <= boundary => 0,
            None => return Err(NoCheckpoint),
        };
        Ok(Playback::new(self, index))
    }
}

impl<S: Snapshot, H: EventStore<Time = S::Time>> Store<S, H> {
    /// Removes obsolete storage, preserving the forwarded checkpoint and all
    /// events at or after the horizon. Does not apply events or change boundaries.
    pub fn prune(&mut self) {
        self.events.prune_before(&self.horizon);
        let before = self.checkpoints.partition_point(|checkpoint| checkpoint.snapshot.time() < &self.horizon);
        self.checkpoints.drain(..before.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    use crate::types::testing::{TestEvent, TestEventStore, TestSnapshot, Time};

    #[rstest::rstest]
    #[case::zero_time(0, 0)]
    #[case::nonzero_time(10, 3)]
    fn happy(#[case] expected_time: Time, #[case] expected_interval: u64) {
        let expected_events = vec![TestEvent(1, 1), TestEvent(2, 2), TestEvent(3, 3)];
        let expected_checkpoints = 1;
        let expected_count = 0;

        let events = TestEventStore(expected_events.clone());
        let snapshot = TestSnapshot { time: expected_time, sum: 0 };

        let mut store = Store::new(events, snapshot, expected_interval);
        let actual_snapshot = store.play(&expected_time).unwrap().checkpoint.snapshot;
        let actual_events = &store.events.0;
        let actual_checkpoints = store.checkpoints.len();
        let actual_count = store.checkpoints[0].history_event_count;
        let actual_interval = store.checkpoint_interval;
        let actual_dirty = store.dirty;
        let actual_horizon = store.horizon;

        assert_eq!(actual_snapshot, TestSnapshot { time: expected_time, sum: 0 });
        assert_eq!(actual_events, &expected_events);
        assert_eq!(actual_checkpoints, expected_checkpoints);
        assert_eq!(actual_count, expected_count);
        assert_eq!(actual_interval, expected_interval);
        assert_eq!(actual_dirty, expected_time);
        assert_eq!(actual_horizon, expected_time);
    }

    #[rstest::rstest]
    #[case::before_horizon(9, false)]
    #[case::empty_checkpoints(10, true)]
    fn rejected_playback_returns_no_checkpoint(#[case] requested_time: Time, #[case] empty: bool) {
        let expected_error = NoCheckpoint;

        let horizon: Time = 10;
        let interval = 1;
        let mut store = Store::new(TestEventStore::default(), TestSnapshot { time: horizon, sum: 0 }, interval);
        if empty {
            store.checkpoints.clear();
        }

        let actual_error = store.play(&requested_time).err();

        assert_eq!(actual_error, Some(expected_error));
    }

    #[rstest::rstest]
    #[case::before(9, false)]
    #[case::at(10, true)]
    #[case::after(11, true)]
    fn updated_horizon_controls_admission(#[case] requested: Time, #[case] expected_allowed: bool) {
        let expected_checkpoint_time: Time = 9;

        let horizon: Time = 10;
        let interval = 1;
        let mut store = Store::new(TestEventStore::default(), TestSnapshot { time: expected_checkpoint_time, sum: 0 }, interval);
        store.horizon = horizon;

        let actual_allowed = store.play(&requested).is_ok();

        assert_eq!(actual_allowed, expected_allowed);
        assert_eq!(*store.checkpoints[0].snapshot.time(), expected_checkpoint_time);
    }

    #[test]
    fn horizon_starts_before_its_timestamp() {
        let expected_time: Time = 9;

        let horizon: Time = 10;
        let mut store = Store::new(TestEventStore::default(), TestSnapshot { time: 0, sum: 0 }, 1);
        store.checkpoints.extend([9, 10, 20].map(|time| Checkpoint { snapshot: TestSnapshot { time, sum: 0 }, history_event_count: time }));
        store.horizon = horizon;
        store.dirty = 20;

        let actual_time = *store.play(&horizon).unwrap().checkpoint.snapshot.time();

        assert_eq!(actual_time, expected_time);
    }

    #[rstest::rstest]
    #[case::before_first_applied(5, 30, 0)]
    #[case::exact(20, 30, 20)]
    #[case::between(25, 30, 20)]
    #[case::dirty_future(40, 10, 10)]
    fn excludes_checkpoints_after_start_or_valid_boundary(#[case] start: Time, #[case] dirty: Time, #[case] expected_time: Time) {
        // The expected time is supplied by each case.

        let interval = 2;
        let initial_time: Time = 0;
        let mut store = Store::new(TestEventStore::default(), TestSnapshot { time: initial_time, sum: 0 }, interval);
        store
            .checkpoints
            .extend([10, 20, 30].map(|time| Checkpoint { snapshot: TestSnapshot { time, sum: 0 }, history_event_count: time / 10 }));
        store.dirty = dirty;

        let playback = store.play(&start).unwrap();
        let actual_time = *playback.checkpoint.snapshot.time();

        assert_eq!(actual_time, expected_time);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_store_unit() {
        let start: Time = 10;
        let interval = 100;
        let checkpoint_count = 1024;
        let mut populated = Store::new(TestEventStore::default(), TestSnapshot { time: start, sum: 0 }, interval);
        populated.checkpoints.extend(
            (1..checkpoint_count)
                .map(|index| Checkpoint { snapshot: TestSnapshot { time: start + index, sum: 0 }, history_event_count: index * interval }),
        );
        let target = start + checkpoint_count - 1;
        populated.dirty = target;
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        criterion.bench_function("snapshots/unit/store/new_and_drop", |b| {
            b.iter(|| {
                black_box(Store::new(
                    black_box(TestEventStore::default()),
                    black_box(TestSnapshot { time: start, sum: 0 }),
                    black_box(interval),
                ));
            });
        });
        criterion.bench_function("snapshots/unit/store/play_1024_checkpoints", |b| {
            b.iter(|| {
                black_box(black_box(&mut populated).play(black_box(&target)).unwrap());
            });
        });
        criterion.final_summary();
    }

    #[rstest::rstest]
    #[case::once(1)]
    #[case::repeated(2)]
    fn prune_preserves_the_predecessor_and_horizon_events(#[case] calls: usize) {
        let expected_times = [19, 30];
        let expected_events = vec![TestEvent(20, 3), TestEvent(20, 4), TestEvent(30, 5)];
        let expected_dirty = 10;
        let expected_horizon = 20;
        let expected_count = 7;

        let events = TestEventStore(vec![TestEvent(5, 1), TestEvent(19, 2), TestEvent(20, 3), TestEvent(20, 4), TestEvent(30, 5)]);
        let mut store = Store::new(events, TestSnapshot { time: 0, sum: 0 }, 100);
        store.checkpoints.extend(
            [9, 19, 19, 30].map(|time| Checkpoint { snapshot: TestSnapshot { time, sum: 3 }, history_event_count: expected_count }),
        );
        store.horizon = expected_horizon;
        store.dirty = expected_dirty;

        for _ in 0..calls {
            store.prune();
        }
        let actual_times = store.checkpoints.iter().map(|checkpoint| checkpoint.snapshot.time).collect::<Vec<_>>();
        let actual_count = store.checkpoints[0].history_event_count;

        assert_eq!(actual_times, expected_times);
        assert_eq!(store.events.0, expected_events);
        assert_eq!(actual_count, expected_count);
        assert_eq!(store.horizon, expected_horizon);
        assert_eq!(store.dirty, expected_dirty);
    }

    #[test]
    fn prune_before_forwarding_preserves_initial_state() {
        let expected_snapshot = TestSnapshot { time: 0, sum: 0 };
        let expected_events = vec![TestEvent(0, 1), TestEvent(10, 2)];
        let expected_len = 1;

        let mut store = Store::new(TestEventStore(expected_events.clone()), expected_snapshot.clone(), 100);

        store.prune();

        assert_eq!(store.checkpoints.len(), expected_len);
        assert_eq!(store.checkpoints[0].snapshot, expected_snapshot);
        assert_eq!(store.events.0, expected_events);
    }

    #[test]
    fn prune_keeps_the_predecessor_when_all_events_are_expired() {
        let expected_snapshot = TestSnapshot { time: 29, sum: 7 };
        let expected_count = 3;
        let expected_len = 1;
        let expected_horizon = 30;
        let expected_dirty = 20;

        let events = TestEventStore(vec![TestEvent(10, 1), TestEvent(20, 2), TestEvent(20, 4)]);
        let mut store = Store::new(events, TestSnapshot { time: 0, sum: 0 }, 100);
        store.checkpoints.push_back(Checkpoint { snapshot: expected_snapshot.clone(), history_event_count: expected_count });
        store.horizon = expected_horizon;
        store.dirty = expected_dirty;

        store.prune();
        let actual_events_empty = store.events.0.is_empty();
        let actual = &store.checkpoints[0];

        assert!(actual_events_empty);
        assert_eq!(actual.snapshot, expected_snapshot);
        assert_eq!(actual.history_event_count, expected_count);
        assert_eq!(store.checkpoints.len(), expected_len);
        assert_eq!(store.horizon, expected_horizon);
        assert_eq!(store.dirty, expected_dirty);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_store_prune_unit() {
        let interval = 100;
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        for event_count in [1000, 10_000] {
            let horizon = event_count / 2 + 1;
            let name = format!("snapshots/unit/store/prune_half_of_{event_count}_events");
            criterion.bench_function(&name, |b| {
                b.iter_batched_ref(
                    || {
                        let events = TestEventStore((1..=event_count).map(|time| TestEvent(time, 1)).collect());
                        let mut store = Store::new(events, TestSnapshot { time: 0, sum: 0 }, interval);
                        store.checkpoints.extend((1..=event_count / interval).map(|index| {
                            let time = index * interval;
                            Checkpoint { snapshot: TestSnapshot { time, sum: time }, history_event_count: time }
                        }));
                        store.horizon = horizon;
                        store.dirty = event_count;
                        store
                    },
                    |store| {
                        black_box(&mut *store).prune();
                        black_box((&store.events.0, &store.checkpoints, store.horizon, store.dirty));
                    },
                    criterion::BatchSize::SmallInput,
                );
            });
        }
        criterion.final_summary();
    }
}
