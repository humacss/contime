use std::marker::PhantomData;

use contime_checkpoints::{ApplyEvents, ApplyWrapper, Snapshot};
use contime_memory::ConservativeTrackedSize;

use crate::{ConTime, ConTimeConfig, Input, MemoryBudget, RouterProcess, WorkerProcess};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I> + ConservativeTrackedSize + Send + 'static,
    W: ApplyWrapper<S, I> + Clone + Send + 'static,
{
    pub fn start(config: ConTimeConfig<I::Time>, wrapper: W) -> Result<Self, contime_runtime::StartError> {
        let budget = MemoryBudget::new(config.memory_limit, config.memory_buffer);
        let (input, incoming) = crossbeam_channel::unbounded();
        let incoming = std::sync::Arc::new(incoming);
        let (error_sender, errors) = crossbeam_channel::unbounded();
        let mut controls = Vec::new();
        let mut subscriptions = Vec::new();
        let mut activity = || {
            let (sender, registrations) = crossbeam_channel::unbounded();
            subscriptions.push(sender);
            registrations
        };
        let routers = (0..config.router_count)
            .map(|_| {
                let mut router = RouterProcess::<I, S>::new(config.placement);
                router.activity = activity();
                let (sender, receiver) = crossbeam_channel::unbounded();
                controls.push(sender);
                router.controls = receiver;
                router
            })
            .collect();
        let workers = (0..config.worker_count)
            .map(|index| {
                let mut worker = WorkerProcess::new(
                    config.worker,
                    config.checkpoints,
                    config.history_retention.clone(),
                    budget.clone(),
                    wrapper.clone(),
                );
                worker.activity = activity();
                let reports = input.clone();
                worker.coordination = Some(contime_worker::Coordination {
                    router_count: config.router_count,
                    report: Box::new(move |round, minimum| {
                        let _ = reports.send(crate::RouterMessage::Report { round, worker: index, minimum });
                    }),
                });
                worker
            })
            .collect();
        let runtime = contime_runtime::Runtime::start(routers, workers)?;
        let mut queues = runtime.queue_checks().to_vec();
        let weak = std::sync::Arc::downgrade(&incoming);
        queues.push(std::sync::Arc::new(move || weak.upgrade().is_none_or(|queue| queue.is_empty())));
        let registrations = activity();
        let output = runtime.input().clone();
        let coordinator = match std::thread::Builder::new().name("contime-admission".into()).spawn(move || {
            crate::coordinator::run(incoming, output, controls, registrations, config.history_retention, config.worker_count);
        }) {
            Ok(coordinator) => coordinator,
            Err(source) => {
                runtime.shutdown();
                return Err(contime_runtime::StartError::ThreadSpawn { stage: contime_runtime::RuntimeStage::Admission, source });
            }
        };
        Ok(Self { runtime, input, coordinator, errors, error_sender, budget, subscriptions, queues, types: PhantomData })
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::Duration;

    use contime_checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
    use contime_memory::ConservativeTrackedSize;
    use criterion::Criterion;

    use crate::{ConTime, ConTimeConfig, Input};

    struct TestInput;

    impl ConservativeTrackedSize for TestInput {
        fn conservative_tracked_size(&self) -> usize {
            1
        }
    }

    impl Input for TestInput {
        type Time = i64;

        fn event_id(&self) -> u128 {
            1
        }

        fn time(&self) -> Self::Time {
            0
        }

        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(1);
        }
    }

    #[derive(Clone, Default)]
    struct TestSnapshot;

    impl ConservativeTrackedSize for TestSnapshot {
        fn conservative_tracked_size(&self) -> usize {
            1
        }
    }

    impl Snapshot for TestSnapshot {
        type Time = i64;

        fn set_time(&mut self, _time: Self::Time) {}
    }

    impl ApplyEvents<TestInput> for TestSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestInput) -> Self {
            Self
        }

        fn apply_events(&mut self, _batch: ApplyBatch<'_, Self::Time, TestInput>) {}
    }

    fn config(router_count: usize, worker_count: usize) -> ConTimeConfig<i64> {
        ConTimeConfig {
            router_count,
            worker_count,
            placement: contime_router::Placement::default(),
            memory_limit: 1_000_000,
            memory_buffer: 1_000,
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
    fn start_rejects_an_empty_router_collection() {
        let result = ConTime::<TestInput, TestSnapshot, ()>::start(config(0, 1), ());

        assert!(matches!(result, Err(contime_runtime::StartError::NoRouters)));
    }

    #[test]
    fn exposed_budget_is_the_admission_budget() {
        use contime_memory::{SizeDelta, TrackedMemoryBudget};
        let core = ConTime::<TestInput, TestSnapshot, ()>::start(config(1, 1), ()).unwrap();
        let external = core.memory_budget();
        let bytes = config(1, 1).memory_limit;
        external.apply_delta(SizeDelta::Increase(bytes));
        assert_eq!(core.memory_budget().used(), bytes);
        assert!(!core.memory_budget().can_admit(1));
        external.apply_delta(SizeDelta::Decrease(bytes));
        assert_eq!(core.memory_budget().used(), 0);
        let _ = core.shutdown();
    }

    #[test]
    fn start_rejects_an_empty_worker_collection() {
        let result = ConTime::<TestInput, TestSnapshot, ()>::start(config(1, 0), ());

        assert!(matches!(result, Err(contime_runtime::StartError::NoWorkers)));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_start() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/start/one_router_one_worker", |bencher| {
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let started = std::time::Instant::now();
                    let contime = ConTime::<TestInput, TestSnapshot, ()>::start(black_box(config(1, 1)), ()).unwrap();
                    measured += started.elapsed();
                    black_box(contime.shutdown());
                }
                measured
            });
        });
        criterion.final_summary();
    }
}
