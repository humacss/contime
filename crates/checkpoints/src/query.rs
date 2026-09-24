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

    use crate::types::testing::{TestEvent, TestEventStore, TestSnapshot, Time};

    #[rstest::rstest]
    #[case::before_events(5, 0, 0)]
    #[case::inclusive_target(20, 20, 6)]
    #[case::between_events(25, 20, 6)]
    #[case::past_events(100, 30, 10)]
    fn happy(#[case] target: Time, #[case] expected_time: Time, #[case] expected_sum: u64) {
        let expected = TestSnapshot { time: expected_time, sum: expected_sum };
        let expected_stored = TestSnapshot { time: 0, sum: 0 };
        let expected_checkpoint_count = 1;
        let expected_event_count = 0;
        let expected_boundary: Time = 0;

        let events = TestEventStore(vec![TestEvent(10, 1), TestEvent(20, 2), TestEvent(20, 3), TestEvent(30, 4)]);
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
        let expected = TestSnapshot { time: 10, sum: 7 };

        let target: Time = 20;
        let mut store = Store::new(TestEventStore(Vec::new()), expected.clone(), 1);

        let actual = query_at(&mut store, &(), target).unwrap();

        assert_eq!(*actual, expected);
    }

    #[test]
    fn rejected_playback_propagates_the_error() {
        let expected_error = NoCheckpoint;

        let horizon: Time = 10;
        let target = horizon - 1;
        let mut store = Store::new(TestEventStore(Vec::new()), TestSnapshot { time: horizon, sum: 0 }, 1);

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
                let name = format!("checkpoints/query/{event_count}_events_{}_intervals", event_count / interval);
                criterion.bench_function(&name, |b| {
                    b.iter_batched_ref(
                        || {
                            let events = TestEventStore((1..=event_count).map(|time| TestEvent(time, 1)).collect());
                            Store::new(events, TestSnapshot { time: 0, sum: 0 }, interval)
                        },
                        |store| {
                            let result = query_at(black_box(store), black_box(&()), black_box(event_count)).unwrap();
                            black_box(result);
                        },
                        criterion::BatchSize::SmallInput,
                    );
                });
            }
        }
        criterion.final_summary();
    }
}
