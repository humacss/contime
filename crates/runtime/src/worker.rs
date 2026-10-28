use std::convert::Infallible;
use std::marker::PhantomData;

use crate::contime_runtime::checkpoints::{ApplyEvents, ApplyWrapper, CheckpointConfig, Snapshot};

use crossbeam_channel::Receiver;

use crate::contime_runtime::types::{CheckpointStorage, CheckpointStorageConfig};
use crate::contime_runtime::{WorkerMessage, WorkerProcess};

impl<I, S, W> WorkerProcess<I, S, W>
where
    I: crate::contime_runtime::ThreadInput,
    S: Snapshot,
{
    /// Registers idle observers for this worker.
    pub fn with_activity(mut self, activity: Receiver<crossbeam_channel::Sender<bool>>) -> Self {
        self.activity = activity;
        self
    }

    /// Connects fenced progress and completed-pruning reports.
    pub fn with_coordination(mut self, coordination: crate::contime_worker::Coordination<I::Time>) -> Self {
        self.coordination = Some(coordination);
        self
    }

    /// Creates a worker adapter with checkpoint spacing and application hooks.
    ///
    /// The worker creates each history lazily on its first accepted input.
    /// Progress coordination and activity observers are initially absent.
    pub fn new(checkpoints: CheckpointConfig, wrapper: W) -> Self {
        Self { checkpoints, wrapper, activity: crossbeam_channel::never(), coordination: None, types: PhantomData }
    }
}

impl<I, S, W> crate::contime_runtime::Worker for WorkerProcess<I, S, W>
where
    I: crate::contime_runtime::ThreadInput,
    S: Snapshot<Time = I::Time> + ApplyEvents<I> + Send + 'static,
    W: ApplyWrapper<S, I> + Send + 'static,
{
    type Input = WorkerMessage<I, S>;
    type Error = Infallible;

    fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
        let checkpoint_config = CheckpointStorageConfig { checkpoints: self.checkpoints };
        crate::contime_worker::work_messages::<WorkerMessage<I, S>, CheckpointStorage<I, S, W>>(
            input,
            checkpoint_config,
            self.wrapper,
            self.activity,
            self.coordination,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::contime_runtime::types::testing::Time;
    use std::hint::black_box;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::contime_api::RejectionMessage;
    use crate::contime_runtime::checkpoints::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, CheckpointConfig, EventBatch, Snapshot};

    use crate::contime_router::{RouteOutput, WorkerOutput};
    use crate::contime_runtime::Worker as RuntimeWorker;
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, TryRecvError};

    use crate::contime_runtime::input::prepare_inputs;
    use crate::contime_runtime::{CompletionHandle, Input, RejectionReason, Route, WorkerBatch, WorkerMessage, WorkerProcess};

    struct TestInput {
        id: u128,
        value: usize,
    }

    impl crate::contime_checkpoints::Event for TestInput {
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

    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: Time,
        value: usize,
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

    impl ApplyEvents<TestInput> for TestSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestInput) -> Self {
            Self::default()
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Self::Time, TestInput>) {
            self.value += batch.events.map(|event| event.value).sum::<usize>();
        }
    }

    #[derive(Clone)]
    struct RecordingWrapper(Arc<AtomicUsize>);

    impl ApplyWrapper<TestSnapshot, TestInput> for RecordingWrapper {
        fn apply_event_batch(&mut self, batch: EventBatch<'_, '_, Time, TestInput>, apply_inner: &mut ApplyInner<'_, TestSnapshot>) {
            apply_inner.apply_event_batch(batch);
            self.0.store(apply_inner.snapshot().value, Ordering::Relaxed);
        }
    }

    fn process(observed: Arc<AtomicUsize>) -> WorkerProcess<TestInput, TestSnapshot, RecordingWrapper> {
        WorkerProcess::new(CheckpointConfig { interval: 100 }, RecordingWrapper(observed))
    }

    fn batch(count: u128) -> (WorkerBatch<TestInput>, crossbeam_channel::Receiver<RejectionMessage<RejectionReason>>) {
        let events = prepare_inputs((0..count).map(|id| TestInput { id, value: 1 }).collect());
        let routes = events.into_iter().map(|event| <Route<TestInput> as RouteOutput<_>>::create(7, event)).collect();
        let (sender, receiver) = unbounded();
        let batch = <WorkerBatch<TestInput> as WorkerOutput<_, _>>::create(routes, CompletionHandle::new(sender));
        (batch, receiver)
    }

    #[test]
    fn worker_process_inserts_replays_and_completes_one_batch() {
        let observed = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = unbounded();
        let (batch, rejections) = batch(5);
        sender.send(WorkerMessage::Apply(batch)).unwrap();
        sender.send(WorkerMessage::Advance(crate::contime_runtime::Advance { time: Time(10), completion: unbounded().0 })).unwrap();
        drop(sender);

        RuntimeWorker::run(process(Arc::clone(&observed)), receiver).unwrap();

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
                    let observed = Arc::new(AtomicUsize::new(0));
                    let (sender, receiver) = unbounded();
                    sender.send(WorkerMessage::Apply(batch(1_000).0)).unwrap();
                    sender
                        .send(WorkerMessage::Advance(crate::contime_runtime::Advance { time: Time(10), completion: unbounded().0 }))
                        .unwrap();
                    drop(sender);
                    (process(observed), receiver)
                },
                |(worker, receiver)| {
                    RuntimeWorker::run(worker, receiver).unwrap();
                    black_box(())
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
