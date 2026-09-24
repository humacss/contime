use crate::{Apply, Checkpoint, Event, EventStore, NoCheckpoint, Snapshot, Store};

/// Reconstructs fresh state without retaining it. Supply an effect-free context.
/// Includes events at the requested time. The returned timestamp remains the
/// last applied timestamp (or the starting snapshot's time if nothing applied).
pub fn query_at<S, H, C>(store: &mut Store<S, H>, context: &C, time: S::Time) -> Result<Box<S>, NoCheckpoint>
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
        playback.commit(|_| {});
    }
    Ok(Box::new(playback.checkpoint.snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hint::black_box;

    type Time = u64;
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct State {
        time: Time,
        sum: u64,
    }
    struct Input(Time, u64);
    struct Events(Vec<Input>);

    impl Snapshot for State {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.time
        }
        fn set_time(&mut self, time: Time) {
            self.time = time;
        }
    }
    impl Event for Input {
        type Time = Time;
        fn time(&self) -> Time {
            self.0
        }
    }
    impl EventStore for Events {
        type Time = Time;
        type Event = Input;
        type Iter<'a> = std::slice::Iter<'a, Input>;
        fn iter_after(&self, boundary: Option<&Time>) -> Self::Iter<'_> {
            let start = boundary.map_or(0, |time| self.0.partition_point(|event| event.0 <= *time));
            self.0[start..].iter()
        }
    }
    impl Apply<Checkpoint<State>> for Input {
        fn apply<'a>(checkpoint: &mut Checkpoint<State>, events: impl Iterator<Item = &'a Self>, _: &()) {
            checkpoint.snapshot.sum += events.map(|event| event.1).sum::<u64>();
        }
    }

    #[rstest::rstest]
    #[case::before_events(5, 0, 0)]
    #[case::inclusive_target(20, 20, 6)]
    #[case::between_events(25, 20, 6)]
    #[case::past_events(100, 30, 10)]
    fn happy(#[case] target: Time, #[case] expected_time: Time, #[case] expected_sum: u64) {
        let expected = State { time: expected_time, sum: expected_sum };
        let expected_stored = State { time: 0, sum: 0 };
        let expected_checkpoint_count = 1;
        let expected_event_count = 0;
        let expected_boundary: Time = 0;

        let events = Events(vec![Input(10, 1), Input(20, 2), Input(20, 3), Input(30, 4)]);
        let interval = 1;
        let mut store = Store::new(events, expected_stored.clone(), interval);

        let actual = query_at(&mut store, &(), target).unwrap();
        let actual_stored = &store.checkpoints[0];

        assert_eq!(*actual, expected);
        assert_eq!(actual_stored.snapshot, expected_stored);
        assert_eq!(actual_stored.history_event_count, expected_event_count);
        assert_eq!(store.checkpoints.len(), expected_checkpoint_count);
        assert_eq!(store.dirty, expected_boundary);
        assert_eq!(store.horizon, expected_boundary);
    }

    #[test]
    fn empty_history_returns_the_initial_state() {
        let expected = State { time: 10, sum: 7 };

        let target: Time = 20;
        let mut store = Store::new(Events(Vec::new()), expected.clone(), 1);

        let actual = query_at(&mut store, &(), target).unwrap();

        assert_eq!(*actual, expected);
    }

    #[test]
    fn rejected_playback_propagates_the_error() {
        let expected_error = NoCheckpoint;

        let horizon: Time = 10;
        let target = horizon - 1;
        let mut store = Store::new(Events(Vec::new()), State { time: horizon, sum: 0 }, 1);

        let actual = query_at(&mut store, &(), target);

        assert_eq!(actual, Err(expected_error));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_query_unit() {
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        for event_count in [1000, 10_000] {
            for interval in [10, 100] {
                let events = Events((1..=event_count).map(|time| Input(time, 1)).collect());
                let mut store = Store::new(events, State { time: 0, sum: 0 }, interval);
                let name = format!("checkpoints/query/{event_count}_events_{}_intervals", event_count / interval);
                criterion.bench_function(&name, |b| {
                    b.iter(|| {
                        let result = query_at(black_box(&mut store), black_box(&()), black_box(event_count)).unwrap();
                        black_box(result);
                    });
                });
            }
        }
        criterion.final_summary();
    }
}
