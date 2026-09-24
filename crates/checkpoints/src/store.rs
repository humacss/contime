//! Storage ownership and playback initialization.
use super::{Checkpoint, Playback};
use crate::{NoCheckpoint, Snapshot};
use std::collections::VecDeque;

/// Storage for one snapshot. Dirty is the inclusive valid-through boundary.
/// Commit callbacks own checkpoint retention, event removal, and validity updates.
pub struct Store<S: Snapshot, H> {
    pub(super) events: H,
    pub(super) checkpoints: VecDeque<Checkpoint<S>>,
    /// Zero leaves event intervals unbounded.
    pub(super) checkpoint_interval: u64,
    pub(super) dirty: S::Time,
    /// Completed pruning boundary; queries and event edits before it are rejected.
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

    /// Starts playback from the latest valid checkpoint at or before start.
    pub fn play(&mut self, start: &S::Time) -> Result<Playback<'_, S, H>, NoCheckpoint> {
        play_with::<Playback<'_, S, H>, _, _>(self, start)
    }
}

// Static dependency seam: Store tests replace initialization, not Store's checks.
trait Initialize<S: Snapshot, H> {
    fn initialize<'a>(store: &'a mut Store<S, H>, start: &S::Time) -> Result<Playback<'a, S, H>, NoCheckpoint>;
}

impl<S: Snapshot, H> Initialize<S, H> for Playback<'_, S, H> {
    fn initialize<'a>(store: &'a mut Store<S, H>, start: &S::Time) -> Result<Playback<'a, S, H>, NoCheckpoint> {
        let index = crate::playback::starting_checkpoint(store, start)?;
        Ok(Playback::new(store, index))
    }
}

#[inline]
fn play_with<'a, P: Initialize<S, H>, S: Snapshot, H>(
    store: &'a mut Store<S, H>,
    start: &S::Time,
) -> Result<Playback<'a, S, H>, NoCheckpoint> {
    if store.checkpoints.is_empty() || start < &store.horizon {
        return Err(NoCheckpoint);
    }
    P::initialize(store, start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::hint::black_box;

    type Time = u64;
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct State(Time);
    #[derive(Default)]
    struct Events {
        values: Vec<u64>,
        initializations: Cell<u64>,
        requested_time: Cell<Time>,
    }
    struct PlaybackStub;

    impl Snapshot for State {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.0
        }
        fn set_time(&mut self, time: Time) {
            self.0 = time;
        }
    }
    impl Initialize<State, Events> for PlaybackStub {
        fn initialize<'a>(store: &'a mut Store<State, Events>, start: &Time) -> Result<Playback<'a, State, Events>, NoCheckpoint> {
            store.events.initializations.set(store.events.initializations.get() + 1);
            store.events.requested_time.set(*start);
            let checkpoint = store.checkpoints[0].clone();
            Ok(Playback { checkpoint, store, interval_start_count: 0 })
        }
    }

    #[rstest::rstest]
    #[case::zero_time(0, 0)]
    #[case::nonzero_time(10, 3)]
    fn happy(#[case] expected_time: Time, #[case] expected_interval: u64) {
        let expected_events = vec![1, 2, 3];
        let expected_checkpoints = 1;
        let expected_count = 0;
        let expected_initializations = 1;

        let events = Events { values: expected_events.clone(), ..Events::default() };
        let snapshot = State(expected_time);

        let mut store = Store::new(events, snapshot, expected_interval);
        let actual_snapshot = play_with::<PlaybackStub, _, _>(&mut store, &expected_time).unwrap().checkpoint.snapshot;
        let actual_events = &store.events.values;
        let actual_checkpoints = store.checkpoints.len();
        let actual_count = store.checkpoints[0].history_event_count;
        let actual_interval = store.checkpoint_interval;
        let actual_dirty = store.dirty;
        let actual_horizon = store.horizon;
        let actual_initializations = store.events.initializations.get();
        let actual_requested = store.events.requested_time.get();

        assert_eq!(actual_snapshot, State(expected_time));
        assert_eq!(actual_events, &expected_events);
        assert_eq!(actual_checkpoints, expected_checkpoints);
        assert_eq!(actual_count, expected_count);
        assert_eq!(actual_interval, expected_interval);
        assert_eq!(actual_dirty, expected_time);
        assert_eq!(actual_horizon, expected_time);
        assert_eq!(actual_initializations, expected_initializations);
        assert_eq!(actual_requested, expected_time);
    }

    #[rstest::rstest]
    #[case::before_horizon(9, false)]
    #[case::empty_checkpoints(10, true)]
    fn rejected_playback_does_not_initialize(#[case] requested_time: Time, #[case] empty: bool) {
        let expected_error = NoCheckpoint;
        let expected_initializations = 0;

        let horizon: Time = 10;
        let interval = 1;
        let mut store = Store::new(Events::default(), State(horizon), interval);
        if empty {
            store.checkpoints.clear();
        }

        let actual_error = play_with::<PlaybackStub, _, _>(&mut store, &requested_time).err();
        let actual_initializations = store.events.initializations.get();

        assert_eq!(actual_error, Some(expected_error));
        assert_eq!(actual_initializations, expected_initializations);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_store_unit() {
        let start: Time = 10;
        let interval = 100;
        let mut populated = Store::new(Events::default(), State(start), interval);
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        criterion.bench_function("checkpoints/unit/store/new_and_drop", |b| {
            b.iter(|| {
                black_box(Store::new(black_box(Events::default()), black_box(State(start)), black_box(interval)));
            });
        });
        criterion.bench_function("checkpoints/unit/store/play_stub", |b| {
            b.iter(|| {
                black_box(play_with::<PlaybackStub, _, _>(black_box(&mut populated), black_box(&start)).unwrap());
            });
        });
        criterion.final_summary();
    }
}
