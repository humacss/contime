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

    pub fn used_memory(&self) -> usize {
        self.budget.used()
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use contime_checkpoints::{ApplyBatch as CheckpointBatch, ApplyEvents as CheckpointApply, CheckpointConfig, Snapshot};
    use contime_lanes::{ApplyBatch as LaneBatch, ApplyEvents as LaneApply, ApplyLanes, FilterLanes, Lanes, RawBatch};
    use contime_memory::ConservativeTrackedSize;
    use criterion::Criterion;

    use crate::{ConTime, ConTimeConfig, Input, RejectionReason};

    struct TestInput {
        id: u128,
        value: usize,
        observed: Arc<AtomicUsize>,
    }

    impl ConservativeTrackedSize for TestInput {
        fn conservative_tracked_size(&self) -> usize {
            64
        }
    }

    impl Input for TestInput {
        type Time = i64;

        fn event_id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> Self::Time {
            10
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
        time: i64,
        observed: Option<Arc<AtomicUsize>>,
    }

    impl ConservativeTrackedSize for TestSnapshot {
        fn conservative_tracked_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    impl Snapshot for TestSnapshot {
        type Time = i64;

        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    impl LaneApply<i64, TestLanes> for TestSnapshot {
        fn apply_events<'a>(&mut self, batch: LaneBatch<i64, <TestLanes as Lanes>::Batch<'a>>)
        where
            TestLanes: 'a,
        {
            let total = batch.events.map(|event| event.value).sum();
            self.observed.as_ref().unwrap().fetch_add(total, Ordering::Relaxed);
        }
    }

    impl CheckpointApply<TestInput> for TestSnapshot {
        fn create(_snapshot_id: u128, first_event: &TestInput) -> Self {
            Self { time: 0, observed: Some(Arc::clone(&first_event.observed)) }
        }

        fn apply_events(&mut self, batch: CheckpointBatch<'_, Self::Time, TestInput>) {
            let filtered = contime_lanes::project::<TestLanes, _, _>(RawBatch {
                snapshot_id: batch.snapshot_id,
                time: batch.time,
                history_event_count: batch.history_event_count,
                events: batch.events,
            });
            contime_lanes::apply::<_, _, TestLanes, TestLanes, _>(self, &contime_lanes::PassThrough, filtered);
        }
    }

    fn config(memory_limit: usize, memory_buffer: usize) -> ConTimeConfig<i64> {
        ConTimeConfig {
            router_count: 1,
            worker_count: 1,
            placement: contime_router::Placement::default(),
            memory_limit,
            memory_buffer,
            history_retention: 0,
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
    fn apply_rejects_a_whole_batch_that_does_not_fit() {
        let contime = ConTime::<TestInput, TestSnapshot, ()>::start(config(100, 50), ()).unwrap();
        let observed = Arc::new(AtomicUsize::new(0));

        contime
            .apply(vec![TestInput { id: 1, value: 1, observed: Arc::clone(&observed) }, TestInput { id: 2, value: 1, observed }])
            .unwrap();
        contime.wait_until_idle(Duration::from_secs(2)).unwrap();
        let response = contime.errors().try_iter().collect::<Vec<_>>();

        assert_eq!(response.len(), 2);
        assert_eq!(response[0].reason, RejectionReason::MemoryFull);
        assert_eq!(contime.used_memory(), 0);
        contime.shutdown();
    }

    #[test]
    fn idle_wait_completes_lane_application_and_duplicate_ids_are_no_ops() {
        let contime = ConTime::<TestInput, TestSnapshot, ()>::start(config(100_000, 1_000), ()).unwrap();
        let observed = Arc::new(AtomicUsize::new(0));

        contime.advance_to(10).unwrap();
        contime.apply(vec![TestInput { id: 1, value: 5, observed: Arc::clone(&observed) }]).unwrap();
        contime.wait_until_idle(Duration::from_secs(2)).unwrap();
        assert_eq!(observed.load(Ordering::Relaxed), 5);

        contime.apply(vec![TestInput { id: 1, value: 9, observed }]).unwrap();
        contime.wait_until_idle(Duration::from_secs(2)).unwrap();
        assert!(contime.errors().is_empty());
        assert!(contime.used_memory() > 0);
        contime.shutdown();
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_apply() {
        let mut criterion = Criterion::default();
        let contime = ConTime::<TestInput, TestSnapshot, ()>::start(config(100, 50), ()).unwrap();
        let observed = Arc::new(AtomicUsize::new(0));
        criterion.bench_function("core/apply/prepare_and_reject_1000", |bencher| {
            bencher.iter(|| {
                let inputs = (0..1_000).map(|id| TestInput { id, value: 1, observed: Arc::clone(&observed) }).collect::<Vec<_>>();
                contime.apply(inputs).unwrap();
                black_box(())
            });
        });
        criterion.final_summary();
        contime.shutdown();
    }
}
