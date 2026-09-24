//! Clone Store-selected state and lend event intervals for a working checkpoint.
use super::{Checkpoint, Store};
use crate::{Event, EventStore, Snapshot};

/// A cloned working checkpoint whose commit callback may change retained storage.
/// A no-op callback leaves the store unchanged, including for query playback.
pub struct Playback<'a, S: Snapshot, H> {
    pub checkpoint: Checkpoint<S>,
    pub(super) store: &'a mut Store<S, H>,
    pub(super) interval_start_count: u64,
    pub(super) checkpoint_index: usize,
}

impl<'a, S: Snapshot, H> Playback<'a, S, H> {
    /// Clones the checkpoint selected by Store without changing retained storage.
    pub(crate) fn new(store: &'a mut Store<S, H>, index: usize) -> Self {
        let checkpoint = store.checkpoints[index].clone();
        let interval_start_count = if index > 0
            && (store.checkpoint_interval == 0
                || checkpoint.history_event_count.saturating_sub(store.checkpoints[index - 1].history_event_count)
                    < store.checkpoint_interval)
        {
            store.checkpoints[index - 1].history_event_count
        } else {
            checkpoint.history_event_count
        };
        Self { checkpoint, store, interval_start_count, checkpoint_index: index }
    }
}

impl<S: Snapshot, H: EventStore<Time = S::Time>> Playback<'_, S, H> {
    /// Borrow the working checkpoint and the remaining whole-timestamp interval.
    /// Reborrowing resumes from recorded application progress, not iterator position.
    pub fn begin(&mut self) -> (&mut Checkpoint<S>, impl Iterator<Item = &H::Event>) {
        let events = interval_events(&self.store.events, &self.checkpoint, self.interval_start_count, self.store.checkpoint_interval);
        (&mut self.checkpoint, events)
    }
    /// Delegates storage changes and prepares the next interval.
    /// A no-op policy leaves retained state unchanged.
    pub fn commit<R>(&mut self, policy: impl FnOnce(&mut crate::Commit<'_, S, H>) -> R) -> R {
        let result = crate::commit::commit(self.store, &self.checkpoint, &mut self.checkpoint_index, policy);
        if self.checkpoint.history_event_count.saturating_sub(self.interval_start_count) >= self.store.checkpoint_interval {
            self.interval_start_count = self.checkpoint.history_event_count;
        }
        result
    }
}

/// Borrow only events; the working checkpoint is not captured by the returned iterator.
fn interval_events<'a, S: Snapshot, H: EventStore<Time = S::Time>>(
    events: &'a H,
    checkpoint: &Checkpoint<S>,
    interval_start_count: u64,
    interval: u64,
) -> impl Iterator<Item = &'a H::Event>
where
    S::Time: 'a,
{
    // A zero count represents initial state, including events at time zero.
    let boundary = (checkpoint.history_event_count != 0).then(|| checkpoint.snapshot.time());
    let mut events = events.iter_after(boundary).fuse().peekable();
    let remaining = interval.saturating_sub(checkpoint.history_event_count.saturating_sub(interval_start_count));
    let mut count = 0u64;
    let mut last_time = None;
    std::iter::from_fn(move || {
        let next = events.peek()?;
        let time = next.time();
        if interval != 0 && count >= remaining && last_time.as_ref() != Some(&time) {
            return None;
        }
        last_time = Some(time);
        count = count.saturating_add(1);
        events.next()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    use crate::types::testing::{TestEvent, TestEventStore, TestSnapshot, Time};

    // Synthetic application results keep these tests independent of apply.rs.
    fn apply_stub<'a>(checkpoint: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a TestEvent>) -> Vec<Time> {
        let mut times = Vec::new();
        <TestEvent as crate::Apply<Checkpoint<TestSnapshot>>>::apply(checkpoint, events.inspect(|event| times.push(event.0)), &());
        if let Some(time) = times.last() {
            checkpoint.snapshot.set_time(*time);
            checkpoint.history_event_count += times.len() as u64;
        }
        times
    }

    #[test]
    fn commit_forwards_the_policy_result() {
        let expected_result = 42;
        let expected_calls = 1;
        let expected_empty = true;

        let start: Time = 0;
        let interval = 2;
        let mut store = Store::new(TestEventStore(Vec::new()), TestSnapshot { time: start, sum: 0 }, interval);
        let mut playback = store.play(&start).unwrap();
        let mut calls = 0;

        let actual_result = playback.commit(|_| {
            calls += 1;
            expected_result
        });
        let actual_empty = playback.begin().1.next().is_none();

        assert_eq!(actual_result, expected_result);
        assert_eq!(calls, expected_calls);
        assert_eq!(actual_empty, expected_empty);
    }

    #[rstest::rstest]
    #[case::initial(0, 100, 0)]
    #[case::unfinished_interval(1, 100, 0)]
    #[case::completed_interval(1, 50, 50)]
    #[case::unlimited(1, 0, 0)]
    fn new_resumes_the_selected_checkpoint(#[case] index: usize, #[case] interval: u64, #[case] expected_start_count: u64) {
        let expected_time: Time = if index == 0 { 0 } else { 50 };
        let expected_count = expected_time;
        let expected_checkpoints = 2;

        let initial_time: Time = 0;
        let stored_time: Time = 50;
        let mut store = Store::new(TestEventStore(Vec::new()), TestSnapshot { time: initial_time, sum: 0 }, interval);
        store.checkpoints.push_back(Checkpoint { snapshot: TestSnapshot { time: stored_time, sum: 0 }, history_event_count: stored_time });

        let playback = Playback::new(&mut store, index);
        let actual_time = *playback.checkpoint.snapshot.time();
        let actual_count = playback.checkpoint.history_event_count;
        let actual_start_count = playback.interval_start_count;
        let actual_checkpoints = store.checkpoints.len();

        assert_eq!(actual_time, expected_time);
        assert_eq!(actual_count, expected_count);
        assert_eq!(actual_start_count, expected_start_count);
        assert_eq!(actual_checkpoints, expected_checkpoints);
    }

    #[rstest::rstest]
    #[case::partial(100, 50, 0)]
    #[case::complete(100, 100, 100)]
    #[case::whole_timestamp_exceeds_interval(100, 105, 105)]
    #[case::unlimited(0, 50, 50)]
    fn commit_tracks_only_completed_interval_progress(
        #[case] interval: u64,
        #[case] applied_count: u64,
        #[case] expected_start_count: u64,
    ) {
        let expected_stored_count = 0;

        let initial_time: Time = 0;
        let mut store = Store::new(TestEventStore(Vec::new()), TestSnapshot { time: initial_time, sum: 0 }, interval);
        let mut playback = Playback::new(&mut store, 0);
        playback.checkpoint.history_event_count = applied_count;

        playback.commit(|_| {});
        let actual_start_count = playback.interval_start_count;
        let actual_stored_count = store.checkpoints[0].history_event_count;

        assert_eq!(actual_start_count, expected_start_count);
        assert_eq!(actual_stored_count, expected_stored_count);
    }

    #[rstest::rstest]
    #[case::whole_timestamp(2, &[0, 10, 10], &[20, 30])]
    #[case::exact_interval(3, &[0, 10, 10], &[20, 30])]
    #[case::unlimited(0, &[0, 10, 10, 20, 30], &[])]
    fn happy(#[case] interval: u64, #[case] expected_first: &[Time], #[case] expected_second: &[Time]) {
        let expected_count = 5;
        let expected_time: Time = 30;
        let expected_retained = 1;
        let expected_dirty: Time = 0;

        let initial_time: Time = 0;
        let events = TestEventStore(vec![TestEvent(0, 1), TestEvent(10, 1), TestEvent(10, 1), TestEvent(20, 1), TestEvent(30, 1)]);
        let mut store = Store::new(events, TestSnapshot { time: initial_time, sum: 0 }, interval);

        let mut playback = store.play(&initial_time).unwrap();
        let (checkpoint, events) = playback.begin();
        let actual_first = apply_stub(checkpoint, events);
        playback.commit(|_| {});
        let (checkpoint, events) = playback.begin();
        let actual_second = apply_stub(checkpoint, events);
        playback.commit(|_| {});
        let (checkpoint, events) = playback.begin();
        let actual_count = checkpoint.history_event_count;
        let actual_time = *checkpoint.snapshot.time();
        let actual_remaining = events.count();
        let actual_retained = store.checkpoints.len();
        let actual_dirty = store.dirty;

        assert_eq!(actual_first, expected_first);
        assert_eq!(actual_second, expected_second);
        assert_eq!(actual_count, expected_count);
        assert_eq!(actual_time, expected_time);
        assert_eq!(actual_remaining, 0);
        assert_eq!(actual_retained, expected_retained);
        assert_eq!(actual_dirty, expected_dirty);
    }

    #[test]
    fn partial_application_does_not_skip_looked_ahead_events() {
        let expected_first = [0];
        let expected_second = [10, 10];
        let expected_third = [20];

        let start: Time = 0;
        let cutoff: Time = 0;
        let interval = 2;
        let events = TestEventStore(vec![TestEvent(0, 1), TestEvent(10, 1), TestEvent(10, 1), TestEvent(20, 1)]);
        let mut store = Store::new(events, TestSnapshot { time: start, sum: 0 }, interval);

        let mut playback = store.play(&start).unwrap();
        let (checkpoint, events) = playback.begin();
        let actual_first = apply_stub(checkpoint, events.take_while(|event| event.0 <= cutoff));
        playback.commit(|_| {});
        let (checkpoint, events) = playback.begin();
        let actual_second = apply_stub(checkpoint, events);
        playback.commit(|_| {});
        let (checkpoint, events) = playback.begin();
        let actual_third = apply_stub(checkpoint, events);

        assert_eq!(actual_first, expected_first);
        assert_eq!(actual_second, expected_second);
        assert_eq!(actual_third, expected_third);
    }

    #[test]
    fn dropping_uncommitted_playback_preserves_stored_state() {
        let expected_time: Time = 0;
        let expected_count = 0;

        let initial_time: Time = 0;
        let interval = 1;
        let mut store = Store::new(TestEventStore(vec![TestEvent(10, 1)]), TestSnapshot { time: initial_time, sum: 0 }, interval);

        {
            let mut playback = store.play(&initial_time).unwrap();
            let (checkpoint, events) = playback.begin();
            apply_stub(checkpoint, events);
        }
        let actual_time = *store.checkpoints[0].snapshot.time();
        let actual_count = store.checkpoints[0].history_event_count;

        assert_eq!(actual_time, expected_time);
        assert_eq!(actual_count, expected_count);
    }

    #[test]
    fn consuming_without_applying_does_not_record_progress() {
        let expected_events = [10, 20];
        let expected_count = 0;

        let start: Time = 0;
        let interval = 2;
        let mut store = Store::new(
            TestEventStore(vec![TestEvent(10, 1), TestEvent(20, 1), TestEvent(30, 1)]),
            TestSnapshot { time: start, sum: 0 },
            interval,
        );

        let mut playback = store.play(&start).unwrap();
        playback.begin().1.for_each(drop);
        playback.commit(|_| {});
        let (checkpoint, events) = playback.begin();
        let actual_count = checkpoint.history_event_count;
        let actual_events = events.map(|event| event.0).collect::<Vec<_>>();

        assert_eq!(actual_events, expected_events);
        assert_eq!(actual_count, expected_count);
    }

    #[test]
    fn begin_leaves_horizon_admission_to_store() {
        let expected_events = [10, 20];

        let initial_time: Time = 0;
        let horizon: Time = 30;
        let interval = 2;
        let mut store =
            Store::new(TestEventStore(vec![TestEvent(10, 1), TestEvent(20, 1)]), TestSnapshot { time: initial_time, sum: 0 }, interval);
        store.horizon = horizon;
        let mut playback = Playback::new(&mut store, 0);

        let actual_events = playback.begin().1.map(|event| event.0).collect::<Vec<_>>();

        assert_eq!(actual_events, expected_events);
    }

    #[rstest::rstest]
    #[case::zero_time(0)]
    #[case::nonzero_time(10)]
    fn empty_events_still_provide_the_initial_checkpoint(#[case] expected_time: Time) {
        let expected_events = 0;
        let expected_dirty = expected_time;

        let interval = 2;
        let mut store = Store::new(TestEventStore(vec![]), TestSnapshot { time: expected_time, sum: 0 }, interval);

        let mut playback = store.play(&expected_time).unwrap();
        playback.commit(|_| {});
        let (checkpoint, events) = playback.begin();
        let actual_time = *checkpoint.snapshot.time();
        let actual_events = events.count();
        let actual_dirty = store.dirty;

        assert_eq!(actual_time, expected_time);
        assert_eq!(actual_events, expected_events);
        assert_eq!(actual_dirty, expected_dirty);
    }

    #[test]
    fn begin_exposes_one_interval_without_applying() {
        let expected_events = 100;
        let expected_sum: Time = 5050;
        let expected_applied_count = 0;

        let initial_time: Time = 0;
        let interval = expected_events;
        let events = TestEventStore((1..=200).map(|time| TestEvent(time, time)).collect());
        let mut store = Store::new(events, TestSnapshot { time: initial_time, sum: 0 }, interval);
        let mut playback = Playback::new(&mut store, 0);

        let (checkpoint, events) = playback.begin();
        let (actual_events, actual_sum) = events.fold((0, 0), |(count, sum), event| (count + 1, sum + event.0));
        let actual_applied_count = checkpoint.history_event_count;

        assert_eq!(actual_events, expected_events);
        assert_eq!(actual_sum, expected_sum);
        assert_eq!(actual_applied_count, expected_applied_count);
    }

    #[test]
    #[ignore = "inline Criterion benchmarks"]
    fn benchmark_playback_unit() {
        let initial_time: Time = 0;
        let interval = 100;
        let events = TestEventStore((1..=200).map(|time| TestEvent(time, time)).collect());
        let mut store = Store::new(events, TestSnapshot { time: initial_time, sum: 0 }, interval);
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        criterion.bench_function("snapshots/unit/playback/new", |b| {
            b.iter(|| {
                black_box(Playback::new(black_box(&mut store), black_box(0)));
            });
        });
        let mut playback = Playback::new(&mut store, 0);
        criterion.bench_function("snapshots/unit/playback/begin_100_events", |b| {
            b.iter(|| {
                let (checkpoint, events) = black_box(&mut playback).begin();
                let sum = events.map(|event| black_box(event.1)).sum::<Time>();
                black_box((checkpoint.history_event_count, sum));
            });
        });
        playback.checkpoint.history_event_count = interval;
        criterion.bench_function("snapshots/unit/playback/commit", |b| {
            b.iter(|| {
                playback.interval_start_count = black_box(0);
                black_box(&mut playback).commit(|_| {});
                black_box(playback.interval_start_count);
            });
        });
        criterion.final_summary();
    }
}
