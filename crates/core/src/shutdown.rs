use std::convert::Infallible;

use crate::{ConTime, Input};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    /// Closes admission and joins the topology. When callbacks submit causal
    /// work through a borrowed/shared owner, wait for idle before releasing
    /// that owner and calling this consuming method.
    pub fn shutdown(self) -> contime_runtime::ShutdownReport<contime_router::RouterError, Infallible> {
        let _ = self.input.send(crate::RouterMessage::Shutdown);
        let _ = self.coordinator.join();
        self.runtime.shutdown()
    }
}

#[cfg(test)]
mod tests {
    use crate::types::testing::Time;
    use std::hint::black_box;
    use std::time::{Duration, Instant};

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

    fn runtime() -> ConTime<TestInput, TestSnapshot, ()> {
        ConTime::start(
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
