#[cfg(test)]
mod tests {
    use super::super::model::*;
    use contime::checkpoints::CheckpointConfig;
    use contime::{ConTime, ConTimeConfig, Placement};
    use std::time::Duration;
    #[test]
    fn late_events_and_duplicate_ids_preserve_history_counts() {
        let contime = ConTime::<CounterChanged, Counter, ()>::start(
            ConTimeConfig {
                router_count: 1,
                worker_count: 2,
                placement: Placement::default(),
                pruning_interval: Duration::from_millis(100),
                history_retention: Time(100),
                checkpoints: CheckpointConfig { interval: 1 },
            },
            (),
        )
        .unwrap();
        let event = |id, time, value| CounterChanged { id, time: Time(time), targets: vec![1, 2], value };
        let (response, inserted) = crossbeam_channel::unbounded();
        contime.send([event(2, 20, 3), event(1, 10, 2), event(1, 10, 2)], response).unwrap();
        assert!(inserted.iter().collect::<Vec<_>>().is_empty());
        let states = contime.query_at(Time(20), [1, 2]).unwrap();
        assert_eq!(states.len(), 2);
        for state in states {
            assert_eq!(state.events, 2);
            assert_eq!(state.value, 5);
        }
        let states = contime.query_at(Time(10), [1, 2]).unwrap();
        for state in states {
            assert_eq!(state.events, 1);
            assert_eq!(state.value, 2);
        }
        // Materialize the original prefix before admitting a late change and a retry.
        let (response, inserted) = crossbeam_channel::unbounded();
        contime.send([event(3, 15, 7), event(1, 10, 2)], response).unwrap();
        assert!(inserted.iter().collect::<Vec<_>>().is_empty());
        for (time, count, value) in [(10, 1, 2), (15, 2, 9), (20, 3, 12)] {
            let states = contime.query_at(Time(time), [1, 2]).unwrap();
            assert_eq!(states.len(), 2);
            for state in states {
                assert_eq!(state.events, count);
                assert_eq!(state.value, value);
            }
        }
        assert!(contime.errors().is_empty());
        let report = contime.shutdown();
        assert!(report.workers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
    }
}
