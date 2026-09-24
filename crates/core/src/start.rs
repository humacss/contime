use std::marker::PhantomData;

use crate::checkpoints::{ApplyEvents, ApplyWrapper, Snapshot};

use crate::{ConTime, ConTimeConfig, Input, RouterProcess, WorkerProcess};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I> + Send + 'static,
    W: ApplyWrapper<S, I> + Clone + Send + 'static,
{
    pub fn start(config: ConTimeConfig<I::Time>, wrapper: W) -> Result<Self, contime_runtime::StartError> {
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
                let mut worker = WorkerProcess::new(config.worker, config.checkpoints, config.history_retention.clone(), wrapper.clone());
                worker.activity = activity();
                let reports = input.clone();
                let pruned = input.clone();
                worker.coordination = Some(contime_worker::Coordination {
                    router_count: config.router_count,
                    report: Box::new(move |round, minimum| {
                        let _ = reports.send(crate::RouterMessage::Report { round, worker: index, minimum });
                    }),
                    pruned: Box::new(move |horizon| {
                        let _ = pruned.send(crate::RouterMessage::Pruned { worker: index, horizon });
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
            crate::coordinator::run(
                incoming,
                output,
                controls,
                registrations,
                config.history_retention,
                config.worker_count,
                config.pruning_interval,
            );
        }) {
            Ok(coordinator) => coordinator,
            Err(source) => {
                runtime.shutdown();
                return Err(contime_runtime::StartError::ThreadSpawn { stage: contime_runtime::RuntimeStage::Admission, source });
            }
        };
        Ok(Self { runtime, input, coordinator, errors, error_sender, subscriptions, queues, types: PhantomData })
    }
}

#[cfg(test)]
mod tests {
    use crate::types::testing::Time;
    use std::hint::black_box;
    use std::time::Duration;

    use crate::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};

    use criterion::Criterion;

    use crate::{ConTime, ConTimeConfig, Input};

    struct TestInput;

    impl contime_checkpoints::Event for TestInput {
        type Time = Time;

        fn time(&self) -> Self::Time {
            Time(0)
        }
    }
    impl Input for TestInput {
        fn event_id(&self) -> u128 {
            1
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(1);
        }
    }

    #[derive(Clone, Default)]
    struct TestSnapshot(Time);

    impl Snapshot for TestSnapshot {
        type Time = Time;

        fn time(&self) -> &Self::Time {
            &self.0
        }
        fn set_time(&mut self, time: Self::Time) {
            self.0 = time;
        }
    }

    impl ApplyEvents<TestInput> for TestSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestInput) -> Self {
            Self(Time(0))
        }

        fn apply_events(&mut self, _batch: ApplyBatch<'_, '_, Self::Time, TestInput>) {}
    }

    fn config(router_count: usize, worker_count: usize) -> ConTimeConfig<Time> {
        ConTimeConfig {
            router_count,
            worker_count,
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
    fn start_rejects_an_empty_router_collection() {
        let result = ConTime::<TestInput, TestSnapshot, ()>::start(config(0, 1), ());

        assert!(matches!(result, Err(contime_runtime::StartError::NoRouters)));
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
