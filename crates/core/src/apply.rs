use contime_api::ApiError;

use crate::{ConTime, Input, RejectionReason};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    /// Enqueues external inputs without waiting for admission or replay.
    /// Observe rejected inputs through [`Self::errors`] and use
    /// [`Self::wait_until_idle`] when a processing barrier is needed.
    pub fn apply(&self, inputs: impl IntoIterator<Item = I>) -> Result<(), ApiError> {
        self.send(inputs, self.error_sender.clone())
    }

    /// Shared rejection stream. Receiving an error is not a completion signal.
    pub fn errors(&self) -> &crossbeam_channel::Receiver<crate::RejectionMessage<RejectionReason>> {
        &self.errors
    }
}

#[cfg(test)]
mod tests {
    use crate::types::testing::Time;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::checkpoints::{ApplyBatch as CheckpointBatch, ApplyEvents as CheckpointApply, CheckpointConfig, Snapshot};
    use contime_lanes::{ApplyBatch as LaneBatch, ApplyEvents as LaneApply, ApplyLanes, FilterLanes, Lanes, RawBatch};

    use crate::{ConTime, ConTimeConfig, Input};

    struct TestInput {
        id: u128,
        value: usize,
        observed: Arc<AtomicUsize>,
    }

    impl contime_checkpoints::Event for TestInput {
        type Time = Time;

        fn time(&self) -> Self::Time {
            Time(10)
        }
    }
    impl Input for TestInput {
        fn event_id(&self) -> u128 {
            self.id
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }

    struct TestLanes;

    impl Lanes for TestLanes {
        type Event<'a> = &'a TestInput;
        type Batch<'a> = std::iter::Copied<std::slice::Iter<'a, &'a TestInput>>;
    }

    impl FilterLanes<TestInput> for TestLanes {
        fn project<'a>(events: &'a [&'a TestInput]) -> Self::Batch<'a> {
            events.iter().copied()
        }
    }

    impl ApplyLanes for TestLanes {}

    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: Time,
        observed: Option<Arc<AtomicUsize>>,
    }

    impl Snapshot for TestSnapshot {
        type Time = Time;

        fn time(&self) -> &Self::Time {
            &self.time
        }
        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    impl LaneApply<Time, TestLanes> for TestSnapshot {
        fn apply_events<'a>(&mut self, batch: LaneBatch<Time, <TestLanes as Lanes>::Batch<'a>>)
        where
            TestLanes: 'a,
        {
            let total = batch.events.map(|event| event.value).sum();
            self.observed.as_ref().unwrap().fetch_add(total, Ordering::Relaxed);
        }
    }

    impl CheckpointApply<TestInput> for TestSnapshot {
        fn create(_snapshot_id: u128, first_event: &TestInput) -> Self {
            Self { time: Time(0), observed: Some(Arc::clone(&first_event.observed)) }
        }

        fn apply_events(&mut self, batch: CheckpointBatch<'_, '_, Self::Time, TestInput>) {
            let events = batch.events.collect::<Vec<_>>();
            let filtered = contime_lanes::project::<TestLanes, _, _>(RawBatch {
                snapshot_id: batch.snapshot_id,
                time: batch.time,
                history_event_count: batch.history_event_count,
                events: &events,
            });
            contime_lanes::apply::<_, _, TestLanes, TestLanes, _>(self, &contime_lanes::PassThrough, filtered);
        }
    }

    fn config() -> ConTimeConfig<Time> {
        ConTimeConfig {
            router_count: 1,
            worker_count: 1,
            placement: contime_router::Placement::default(),
            pruning_interval: std::time::Duration::from_millis(100),

            history_retention: Time(0),
            worker: contime_worker::WorkerConfig {
                maximum_dirty_age: Duration::from_micros(100),
                replays_per_receive: 1,
                deadline_compaction_minimum: 1_024,
                deadline_compaction_multiplier: 2,
            },
            checkpoints: CheckpointConfig { interval: 100 },
        }
    }

    #[test]
    fn idle_wait_completes_lane_application_and_duplicate_ids_are_no_ops() {
        let contime = ConTime::<TestInput, TestSnapshot, ()>::start(config(), ()).unwrap();
        let observed = Arc::new(AtomicUsize::new(0));

        contime.advance_to(Time(10)).unwrap();
        contime.apply(vec![TestInput { id: 1, value: 5, observed: Arc::clone(&observed) }]).unwrap();
        contime.wait_until_idle(Duration::from_secs(2)).unwrap();
        assert_eq!(observed.load(Ordering::Relaxed), 5);

        contime.apply(vec![TestInput { id: 1, value: 9, observed }]).unwrap();
        contime.wait_until_idle(Duration::from_secs(2)).unwrap();
        assert!(contime.errors().is_empty());

        contime.shutdown();
    }
}
