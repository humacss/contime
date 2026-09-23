//! Applies the supplied ordered slice in complete timestamp batches.
use crate::{Apply, Checkpoint, Snapshot};

pub(crate) fn apply<S: Snapshot, E: Apply<Checkpoint<S>, C, Time = S::Time>, C>(checkpoint: &mut Checkpoint<S>, events: &[E], context: &C) {
    for batch in events.chunk_by(|left, right| left.time() == right.time()) {
        E::apply(checkpoint, batch, context);
        checkpoint.history_event_count = checkpoint.history_event_count.saturating_add(batch.len() as u64);
        checkpoint.snapshot.set_time(batch[0].time());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Event;

    type Time = i64;

    const CONTEXT: i32 = 10;

    #[derive(Clone)]
    struct State(i32, Time);

    struct Input(Time, i32);
    struct PanickingInput(Time);

    impl Snapshot for State {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.1
        }
        fn set_time(&mut self, time: Time) {
            self.1 = time;
        }
    }

    impl Event for Input {
        type Time = Time;
        fn time(&self) -> Time {
            self.0
        }
    }

    impl Apply<Checkpoint<State>, i32> for Input {
        fn apply(state: &mut Checkpoint<State>, events: &[Self], context: &i32) {
            state.snapshot.0 += events.iter().map(|event| event.1).sum::<i32>() + context;
        }
    }

    impl Event for PanickingInput {
        type Time = Time;
        fn time(&self) -> Time {
            self.0
        }
    }

    impl Apply<Checkpoint<State>, i32> for PanickingInput {
        fn apply(state: &mut Checkpoint<State>, events: &[Self], _: &i32) {
            state.snapshot.0 += events[0].0 as i32;
            panic!("injected application failure");
        }
    }

    #[rstest::rstest]
    #[case::multiple_batches(5, 7, &[Input(10, 3), Input(10, 4), Input(20, 5)], 32, 10, 20)]
    #[case::first_event_at_zero(0, 0, &[Input(0, 3)], 13, 1, 0)]
    #[case::count_overflow(5, u64::MAX - 1, &[Input(10, 3), Input(10, 4), Input(20, 5)], 32, u64::MAX, 20)]
    #[case::count_already_saturated(5, u64::MAX, &[Input(10, 3), Input(10, 4), Input(20, 5)], 32, u64::MAX, 20)]
    fn happy(
        #[case] initial_time: Time,
        #[case] initial_count: u64,
        #[case] events: &[Input],
        #[case] expected_sum: i32,
        #[case] expected_count: u64,
        #[case] expected_time: Time,
    ) {
        // Expected values are supplied by each case.

        let initial_sum = 0;
        let mut state = Checkpoint { snapshot: State(initial_sum, initial_time), history_event_count: initial_count };

        apply(&mut state, events, &CONTEXT);
        let actual_sum = state.snapshot.0;
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
        let mut state = Checkpoint { snapshot: State(initial_sum, initial_time), history_event_count: initial_count };

        apply::<_, Input, _>(&mut state, &[], &CONTEXT);
        let actual_sum = state.snapshot.0;
        let actual_time = *state.snapshot.time();
        let actual_count = state.history_event_count;

        assert_eq!(actual_sum, expected_sum);
        assert_eq!(actual_time, expected_time);
        assert_eq!(actual_count, expected_count);
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
        let mut state = Checkpoint { snapshot: State(initial_sum, initial_time), history_event_count: initial_count };
        let events = [PanickingInput(first_event_time), PanickingInput(later_event_time)];

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply(&mut state, &events, &CONTEXT);
        }));
        let actual_panic = result.is_err();
        let actual_sum = state.snapshot.0;
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
        let events = (1..=1000).map(|time| Input(time, 1)).collect::<Vec<_>>();
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        criterion.bench_function("checkpoints/unit/apply_1000_timestamps/running_sum", |b| {
            b.iter(|| {
                let mut checkpoint = Checkpoint { snapshot: State(0, 0), history_event_count: 0 };
                apply(&mut checkpoint, std::hint::black_box(&events), std::hint::black_box(&CONTEXT));
                std::hint::black_box(checkpoint);
            });
        });
        criterion.final_summary();
    }
}
