use std::convert::Infallible;
use std::marker::PhantomData;

use contime_checkpoints::{ApplyEvents, ApplyWrapper, CheckpointConfig, Snapshot};
use contime_memory::ConservativeTrackedSize;
use crossbeam_channel::Receiver;

use crate::types::{CheckpointStorage, CheckpointStorageConfig, History};
use crate::{Input, MemoryBudget, WorkerMessage, WorkerProcess};

impl<I, S, W> WorkerProcess<I, S, W>
where
    I: Input,
    S: Snapshot + ConservativeTrackedSize,
{
    pub fn new(worker: contime_worker::WorkerConfig, checkpoints: CheckpointConfig, budget: MemoryBudget, wrapper: W) -> Self {
        Self { worker, checkpoints, budget, wrapper, types: PhantomData }
    }
}

impl<I, S, W> contime_runtime::Worker for WorkerProcess<I, S, W>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I> + ConservativeTrackedSize + Send + 'static,
    W: ApplyWrapper<S, I> + Send + 'static,
{
    type Input = WorkerMessage<I, S>;
    type Error = Infallible;

    fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
        let checkpoint_config = CheckpointStorageConfig { checkpoints: self.checkpoints, budget: self.budget };
        contime_worker::work_messages::<WorkerMessage<I, S>, History<I>, CheckpointStorage<S, W>>(
            input,
            self.worker,
            (),
            checkpoint_config,
            self.wrapper,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use contime_api::RejectionMessage;
    use contime_checkpoints::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, CheckpointConfig, EventBatch, Snapshot};
    use contime_memory::ConservativeTrackedSize;
    use contime_router::{RouteOutput, WorkerOutput};
    use contime_runtime::Worker as RuntimeWorker;
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, TryRecvError};

    use crate::input::prepare_inputs;
    use crate::{CompletionHandle, Input, MemoryBudget, RejectionReason, Route, WorkerBatch, WorkerMessage, WorkerProcess};

    struct TestInput {
        id: u128,
        value: usize,
    }

    impl ConservativeTrackedSize for TestInput {
        fn conservative_tracked_size(&self) -> usize {
            32
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

    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: i64,
        value: usize,
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

    impl ApplyEvents<TestInput> for TestSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestInput) -> Self {
            Self::default()
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, TestInput>) {
            self.value += batch.events.iter().map(|event| event.value).sum::<usize>();
        }
    }

    #[derive(Clone)]
    struct RecordingWrapper(Arc<AtomicUsize>);

    impl ApplyWrapper<TestSnapshot, TestInput> for RecordingWrapper {
        fn apply_event_batch(&mut self, batch: EventBatch<'_, i64, TestInput>, apply_inner: &mut ApplyInner<'_, TestSnapshot>) {
            apply_inner.apply_event_batch(batch);
            self.0.store(apply_inner.snapshot().value, Ordering::Relaxed);
        }
    }

    fn process(budget: MemoryBudget, observed: Arc<AtomicUsize>) -> WorkerProcess<TestInput, TestSnapshot, RecordingWrapper> {
        WorkerProcess::new(
            contime_worker::WorkerConfig {
                maximum_dirty_age: Duration::from_micros(100),
                replays_per_receive: 1,
                deadline_compaction_minimum: 1_024,
                deadline_compaction_multiplier: 2,
            },
            CheckpointConfig { interval: 100 },
            budget,
            RecordingWrapper(observed),
        )
    }

    fn batch(
        budget: &MemoryBudget,
        count: u128,
    ) -> (WorkerBatch<TestInput>, crossbeam_channel::Receiver<RejectionMessage<RejectionReason>>) {
        let events = prepare_inputs(budget, (0..count).map(|id| TestInput { id, value: 1 }).collect()).unwrap();
        let routes = events.into_iter().map(|event| <Route<TestInput> as RouteOutput<_>>::create(7, event)).collect();
        let (sender, receiver) = unbounded();
        let batch = <WorkerBatch<TestInput> as WorkerOutput<_, _>>::create(routes, CompletionHandle::new(sender));
        (batch, receiver)
    }

    #[test]
    fn worker_process_inserts_replays_and_completes_one_batch() {
        let budget = MemoryBudget::new(100_000, 1_000);
        let observed = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = unbounded();
        let (batch, rejections) = batch(&budget, 5);
        sender.send(WorkerMessage::Apply(batch)).unwrap();
        drop(sender);

        RuntimeWorker::run(process(budget, Arc::clone(&observed)), receiver).unwrap();

        assert_eq!(observed.load(Ordering::Relaxed), 5);
        assert_eq!(rejections.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_worker() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/worker/1000_events_one_snapshot", |bencher| {
            bencher.iter_batched(
                || {
                    let budget = MemoryBudget::new(usize::MAX, 0);
                    let observed = Arc::new(AtomicUsize::new(0));
                    let (sender, receiver) = unbounded();
                    sender.send(WorkerMessage::Apply(batch(&budget, 1_000).0)).unwrap();
                    drop(sender);
                    (process(budget, observed), receiver)
                },
                |(worker, receiver)| black_box(RuntimeWorker::run(worker, receiver).unwrap()),
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
