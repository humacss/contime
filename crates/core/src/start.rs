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
    pub fn start(config: ConTimeConfig, wrapper: W) -> Result<Self, contime_runtime::StartError> {
        let budget = MemoryBudget::new(config.memory_limit, config.memory_buffer);
        let routers = (0..config.router_count).map(|_| RouterProcess::new(config.router_seed)).collect();
        let workers = (0..config.worker_count)
            .map(|_| WorkerProcess::new(config.worker, config.checkpoints, budget.clone(), wrapper.clone()))
            .collect();
        let runtime = contime_runtime::Runtime::start(routers, workers)?;
        Ok(Self { runtime, budget, types: PhantomData })
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

    fn config(router_count: usize, worker_count: usize) -> ConTimeConfig {
        ConTimeConfig {
            router_count,
            worker_count,
            router_seed: 9,
            memory_limit: 1_000_000,
            memory_buffer: 1_000,
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
