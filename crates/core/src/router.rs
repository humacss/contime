use crossbeam_channel::{Receiver, Sender};

use crate::{Input, RouterBatch, RouterProcess, WorkerBatch};

impl<I> RouterProcess<I>
where
    I: Input,
{
    pub const fn new(seed: u64) -> Self {
        Self { seed, input: std::marker::PhantomData }
    }
}

impl<I> contime_runtime::Router for RouterProcess<I>
where
    I: Input,
{
    type Input = RouterBatch<I>;
    type WorkerInput = WorkerBatch<I>;
    type Error = contime_router::RouterError;

    fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
        contime_router::route(self.seed, input, &workers)
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use contime_api::{ApplyOutput, RejectionMessage};
    use contime_memory::ConservativeTrackedSize;
    use contime_runtime::Router as RuntimeRouter;
    use contime_worker::ApplyInput;
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::unbounded;

    use crate::input::prepare_inputs;
    use crate::{Input, MemoryBudget, RejectionReason, RouterBatch, RouterProcess, WorkerBatch};

    struct TestInput(u128);

    impl ConservativeTrackedSize for TestInput {
        fn conservative_tracked_size(&self) -> usize {
            32
        }
    }

    impl Input for TestInput {
        type Time = i64;

        fn event_id(&self) -> u128 {
            self.0
        }

        fn time(&self) -> Self::Time {
            0
        }

        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(self.0 % 2);
        }
    }

    fn batch(count: u128) -> RouterBatch<TestInput> {
        let budget = MemoryBudget::new(usize::MAX, 0);
        let events = prepare_inputs(&budget, (0..count).map(TestInput).collect()).unwrap();
        let (completion, _rejections) = unbounded::<RejectionMessage<RejectionReason>>();
        <RouterBatch<TestInput> as ApplyOutput<_, _>>::create(events, completion)
    }

    #[test]
    fn router_process_forwards_every_snapshot_route_to_workers() {
        let (input, receiver) = unbounded();
        let (worker, output) = unbounded::<WorkerBatch<TestInput>>();
        input.send(batch(4)).unwrap();
        drop(input);

        RuntimeRouter::run(RouterProcess::new(9), receiver, vec![worker]).unwrap();

        let routed = output.recv().unwrap();
        assert_eq!(routed.into_parts().0.len(), 4);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_router() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/router/1000_routes_one_worker", |bencher| {
            bencher.iter_batched(
                || {
                    let (input, receiver) = unbounded();
                    let (worker, output) = unbounded();
                    input.send(batch(1_000)).unwrap();
                    drop(input);
                    (receiver, worker, output)
                },
                |(receiver, worker, output)| {
                    RuntimeRouter::run(RouterProcess::new(9), receiver, vec![worker]).unwrap();
                    black_box(output.recv().unwrap())
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
