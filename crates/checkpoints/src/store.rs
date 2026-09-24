//! Storage ownership and playback initialization.
use super::{Checkpoint, Playback};
use crate::{NoCheckpoint, Snapshot};
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    type Time = u64;
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct State(Time);
    #[derive(Default)]
    struct Events {
        values: Vec<u64>,
    }

    impl Snapshot for State {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.0
        }
        fn set_time(&mut self, time: Time) {
            self.0 = time;
        }
    }
    #[rstest::rstest]
    #[case::zero_time(0, 0)]
    #[case::nonzero_time(10, 3)]
    fn happy(#[case] expected_time: Time, #[case] expected_interval: u64) {
        let expected_events = vec![1, 2, 3];
        let expected_checkpoints = 1;
        let expected_count = 0;

        let events = Events { values: expected_events.clone() };
        let snapshot = State(expected_time);

        let mut store = Store::new(events, snapshot, expected_interval);
        let actual_snapshot = store.play(&expected_time).unwrap().checkpoint.snapshot;
        let actual_events = &store.events.values;
        let actual_checkpoints = store.checkpoints.len();
        let actual_count = store.checkpoints[0].history_event_count;
        let actual_interval = store.checkpoint_interval;
        let actual_dirty = store.dirty;
        let actual_horizon = store.horizon;

        assert_eq!(actual_snapshot, State(expected_time));
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
        let mut store = Store::new(Events::default(), State(horizon), interval);
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
        let mut store = Store::new(Events::default(), State(expected_checkpoint_time), interval);
        store.horizon = horizon;

        let actual_allowed = store.play(&requested).is_ok();

        assert_eq!(actual_allowed, expected_allowed);
        assert_eq!(*store.checkpoints[0].snapshot.time(), expected_checkpoint_time);
    }

    #[test]
    fn horizon_starts_before_its_timestamp() {
        let expected_time: Time = 9;

        let horizon: Time = 10;
        let mut store = Store::new(Events::default(), State(0), 1);
        store.checkpoints.extend([9, 10, 20].map(|time| Checkpoint { snapshot: State(time), history_event_count: time }));
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
        let mut store = Store::new(Events::default(), State(initial_time), interval);
        store.checkpoints.extend([10, 20, 30].map(|time| Checkpoint { snapshot: State(time), history_event_count: time / 10 }));
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
        let mut populated = Store::new(Events::default(), State(start), interval);
        populated.checkpoints.extend(
            (1..checkpoint_count).map(|index| Checkpoint { snapshot: State(start + index), history_event_count: index * interval }),
        );
        let target = start + checkpoint_count - 1;
        populated.dirty = target;
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        criterion.bench_function("checkpoints/unit/store/new_and_drop", |b| {
            b.iter(|| {
                black_box(Store::new(black_box(Events::default()), black_box(State(start)), black_box(interval)));
            });
        });
        criterion.bench_function("checkpoints/unit/store/play_1024_checkpoints", |b| {
            b.iter(|| {
                black_box(black_box(&mut populated).play(black_box(&target)).unwrap());
            });
        });
        criterion.final_summary();
    }
}
