use std::convert::Infallible;

use crate::{ConTime, Input};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    pub fn shutdown(self) -> contime_runtime::ShutdownReport<contime_router::RouterError, Infallible> {
        self.runtime.shutdown()
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::{Duration, Instant};

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

    fn runtime() -> ConTime<TestInput, TestSnapshot, ()> {
        ConTime::start(
            ConTimeConfig {
                router_count: 1,
                worker_count: 1,
                router_seed: 9,
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
            },
            (),
        )
        .unwrap()
    }

    #[test]
    fn shutdown_joins_every_supplied_process() {
        let report = runtime().shutdown();

        assert_eq!(report.routers, vec![contime_runtime::ThreadOutcome::Completed]);
        assert_eq!(report.workers, vec![contime_runtime::ThreadOutcome::Completed]);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_shutdown() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/shutdown/one_router_one_worker", |bencher| {
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let runtime = runtime();
                    let started = Instant::now();
                    black_box(runtime.shutdown());
                    measured += started.elapsed();
                }
                measured
            });
        });
        criterion.final_summary();
    }
}
