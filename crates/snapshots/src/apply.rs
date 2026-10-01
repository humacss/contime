//! Applies an ordered borrowed iterator in complete timestamp batches.
use crate::{Apply, BatchLookahead, Checkpoint, Event, Snapshot};

pub(crate) fn apply<'a, S: Snapshot, E: Apply<Checkpoint<S>, C, Time = S::Time> + 'a, C>(
    checkpoint: &mut Checkpoint<S>,
    events: impl Iterator<Item = &'a E>,
    context: &C,
) {
    apply_with_lookahead(checkpoint, events, BatchLookahead::unavailable(), context);
}

pub(crate) fn apply_with_lookahead<'a, S: Snapshot, E: Apply<Checkpoint<S>, C, Time = S::Time> + 'a, C>(
    checkpoint: &mut Checkpoint<S>,
    events: impl Iterator<Item = &'a E>,
    after_events: BatchLookahead<'_, E::Time>,
    context: &C,
) {
    let mut events = events.fuse().peekable();
    while let Some(event) = events.peek() {
        let batch_time = event.time();
        let mut batch_count = 0u64;
        let next_event_time = std::cell::RefCell::new(None);
        E::apply(
            checkpoint,
            timestamp_batch(&mut events, &batch_time, &mut batch_count, &next_event_time),
            BatchLookahead::with_fallback(&next_event_time, after_events),
            context,
        );
        checkpoint.history_event_count = checkpoint.history_event_count.saturating_add(batch_count);
        checkpoint.snapshot.set_time(batch_time);
    }
}

#[inline]
fn timestamp_batch<'a: 'b, 'b, E: Event + 'a>(
    events: &'b mut std::iter::Peekable<impl Iterator<Item = &'a E> + 'b>,
    time: &'b E::Time,
    count: &'b mut u64,
    next_event_time: &'b std::cell::RefCell<Option<Option<E::Time>>>,
) -> impl Iterator<Item = &'a E> + 'b {
    TimestampBatch {
        inner: std::iter::from_fn(move || {
            let Some(event) = events.peek() else {
                next_event_time.replace(Some(None));
                return None;
            };
            let event_time = event.time();
            if event_time != *time {
                next_event_time.replace(Some(Some(event_time)));
                return None;
            }
            *count = count.saturating_add(1);
            events.next()
        })
        .fuse(),
    }
}

struct TimestampBatch<I: Iterator> {
    inner: I,
}

impl<I: Iterator> Iterator for TimestampBatch<I> {
    type Item = I::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<I: Iterator> Drop for TimestampBatch<I> {
    #[inline]
    fn drop(&mut self) {
        // Finish this timestamp even when the consumer stops reading early.
        for _ in self.inner.by_ref() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::types::testing::{TestEvent, TestSnapshot, Time};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const CONTEXT: u64 = 10;
    struct PanickingTestEvent(Time);

    impl Apply<Checkpoint<TestSnapshot>, usize> for TestEvent {
        fn apply<'a>(
            state: &mut Checkpoint<TestSnapshot>,
            events: impl Iterator<Item = &'a Self>,
            _: BatchLookahead<'_, Time>,
            limit: &usize,
        ) where
            Self: 'a,
        {
            state.snapshot.sum += events.take(*limit).map(|event| event.1).sum::<u64>();
        }
    }

    impl Event for PanickingTestEvent {
        type Time = Time;
        fn time(&self) -> Time {
            self.0
        }
    }

    impl Apply<Checkpoint<TestSnapshot>, u64> for PanickingTestEvent {
        fn apply<'a>(state: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, _: BatchLookahead<'_, Time>, _: &u64)
        where
            Self: 'a,
        {
            state.snapshot.sum += events.into_iter().next().unwrap().0;
            panic!("injected application failure");
        }
    }

    #[rstest::rstest]
    #[case::multiple_batches(5, 7, &[TestEvent(10, 3), TestEvent(10, 4), TestEvent(20, 5)], 32, 10, 20)]
    #[case::first_event_at_zero(0, 0, &[TestEvent(0, 3)], 13, 1, 0)]
    #[case::count_overflow(5, u64::MAX - 1, &[TestEvent(10, 3), TestEvent(10, 4), TestEvent(20, 5)], 32, u64::MAX, 20)]
    #[case::count_already_saturated(5, u64::MAX, &[TestEvent(10, 3), TestEvent(10, 4), TestEvent(20, 5)], 32, u64::MAX, 20)]
    fn happy(
        #[case] initial_time: Time,
        #[case] initial_count: u64,
        #[case] events: &[TestEvent],
        #[case] expected_sum: u64,
        #[case] expected_count: u64,
        #[case] expected_time: Time,
    ) {
        // Expected values are supplied by each case.

        let initial_sum = 0;
        let mut state = Checkpoint { snapshot: TestSnapshot { sum: initial_sum, time: initial_time }, history_event_count: initial_count };

        apply(&mut state, events.iter(), &CONTEXT);
        let actual_sum = state.snapshot.sum;
        let actual_count = state.history_event_count;
        let actual_time = *state.snapshot.time();

        assert_eq!(actual_sum, expected_sum);
        assert_eq!(actual_count, expected_count);
        assert_eq!(actual_time, expected_time);
    }

    #[rstest::rstest]
    #[case::initial_checkpoint(0)]
    #[case::existing_checkpoint(7)]
    fn empty_slice_does_not_call_apply(#[case] initial_count: u64) {
        let expected_sum = 100;
        let expected_time: Time = 10;
        let expected_count = initial_count;

        let initial_sum = expected_sum;
        let initial_time = expected_time;
        let mut state = Checkpoint { snapshot: TestSnapshot { sum: initial_sum, time: initial_time }, history_event_count: initial_count };

        apply::<_, TestEvent, _>(&mut state, std::iter::empty(), &CONTEXT);
        let actual_sum = state.snapshot.sum;
        let actual_time = *state.snapshot.time();
        let actual_count = state.history_event_count;

        assert_eq!(actual_sum, expected_sum);
        assert_eq!(actual_time, expected_time);
        assert_eq!(actual_count, expected_count);
    }

    #[rstest::rstest]
    #[case::ignores_batch(0, 0)]
    #[case::reads_first_event(1, 8)]
    fn unread_events_are_drained_before_the_next_batch(#[case] limit: usize, #[case] expected_sum: u64) {
        let expected_count = 3;
        let expected_time: Time = 20;

        let initial_sum = 0;
        let initial_time: Time = 0;
        let initial_count = 0;
        let mut state = Checkpoint { snapshot: TestSnapshot { sum: initial_sum, time: initial_time }, history_event_count: initial_count };
        let events = [TestEvent(10, 3), TestEvent(10, 4), TestEvent(20, 5)];

        apply(&mut state, events.iter(), &limit);
        let actual_sum = state.snapshot.sum;
        let actual_count = state.history_event_count;
        let actual_time = *state.snapshot.time();

        assert_eq!(actual_sum, expected_sum);
        assert_eq!(actual_count, expected_count);
        assert_eq!(actual_time, expected_time);
    }

    struct CountingEvent {
        time: Time,
        time_reads: std::sync::Arc<AtomicUsize>,
    }

    impl Event for CountingEvent {
        type Time = Time;

        fn time(&self) -> Time {
            self.time_reads.fetch_add(1, Ordering::Relaxed);
            self.time
        }
    }

    impl Apply<Checkpoint<TestSnapshot>> for CountingEvent {
        fn apply<'a>(_: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, lookahead: BatchLookahead<'_, Time>, _: &())
        where
            Self: 'a,
        {
            assert_eq!(events.count(), 1);
            let _ = lookahead.next_event_time();
        }
    }

    #[test]
    fn timestamp_batch_reads_each_boundary_once() {
        let time_reads = std::sync::Arc::new(AtomicUsize::new(0));
        let events = [10, 20, 30].map(|time| CountingEvent { time, time_reads: time_reads.clone() });
        let mut state = Checkpoint { snapshot: TestSnapshot { time: 0, sum: 0 }, history_event_count: 0 };

        apply(&mut state, events.iter(), &());

        assert_eq!(time_reads.load(Ordering::Relaxed), 8);
    }

    #[test]
    fn application_panic_leaves_timestamp_unchanged_and_skips_later_batches() {
        let expected_panic = true;
        let expected_sum = 10;
        let expected_time: Time = 5;
        let expected_count = 7;

        let initial_sum = 0;
        let initial_time = expected_time;
        let initial_count = expected_count;
        let first_event_time: Time = 10;
        let later_event_time: Time = 20;
        let mut state = Checkpoint { snapshot: TestSnapshot { sum: initial_sum, time: initial_time }, history_event_count: initial_count };
        let events = [PanickingTestEvent(first_event_time), PanickingTestEvent(later_event_time)];

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply(&mut state, events.iter(), &CONTEXT);
        }));
        let actual_panic = result.is_err();
        let actual_sum = state.snapshot.sum;
        let actual_time = *state.snapshot.time();
        let actual_count = state.history_event_count;

        assert_eq!(actual_panic, expected_panic);
        assert_eq!(actual_sum, expected_sum);
        assert_eq!(actual_time, expected_time);
        assert_eq!(actual_count, expected_count);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_apply_unit() {
        let events = (1..=1000).map(|time| TestEvent(time, 1)).collect::<Vec<_>>();
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        criterion.bench_function("snapshots/unit/apply_1000_timestamps/running_sum", |b| {
            b.iter(|| {
                let mut checkpoint = Checkpoint { snapshot: TestSnapshot { time: 0, sum: 0 }, history_event_count: 0 };
                apply(&mut checkpoint, std::hint::black_box(&events).iter(), std::hint::black_box(&()));
                std::hint::black_box(checkpoint);
            });
        });
        criterion.final_summary();
    }
}
