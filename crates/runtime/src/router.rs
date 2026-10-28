use crossbeam_channel::{Receiver, Sender};

use crate::contime_runtime::{RouterMessage, RouterProcess, WorkerMessage};

impl<I, S> RouterProcess<I, S>
where
    I: crate::contime_runtime::ThreadInput,
{
    /// Registers idle observers for this router.
    pub fn with_activity(mut self, activity: Receiver<Sender<bool>>) -> Self {
        self.activity = activity;
        self
    }

    /// Connects fenced flush requests from the admission coordinator.
    pub fn with_controls(mut self, controls: Receiver<crate::contime_router::Flush>) -> Self {
        self.controls = controls;
        self
    }

    /// Creates a router adapter using the supplied deterministic placement.
    ///
    /// Activity observation and flush controls are initially disabled; attach
    /// their receivers with the corresponding builder methods when needed.
    pub fn new(placement: crate::contime_router::Placement) -> Self {
        Self { placement, activity: crossbeam_channel::never(), controls: crossbeam_channel::never(), input: std::marker::PhantomData }
    }
}

impl<I, S> crate::contime_runtime::Router for RouterProcess<I, S>
where
    I: crate::contime_runtime::ThreadInput,
    S: Send + 'static,
{
    type Input = RouterMessage<I, S>;
    type WorkerInput = WorkerMessage<I, S>;
    type Error = crate::contime_router::RouterError;

    fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
        crate::contime_router::route_messages(self.placement, input, &workers, self.activity, self.controls)
    }
}

#[cfg(test)]
mod tests {
    use crate::contime_runtime::types::testing::Time;
    use std::hint::black_box;

    use crate::contime_api::{ApplyOutput, RejectionMessage};

    use crate::contime_runtime::Router as RuntimeRouter;
    use crate::contime_worker::ApplyInput;
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::unbounded;

    use crate::contime_runtime::input::prepare_inputs;
    use crate::contime_runtime::{Input, RejectionReason, RouterBatch, RouterMessage, RouterProcess, WorkerMessage};

    struct TestInput(u128);

    impl crate::contime_checkpoints::Event for TestInput {
        type Time = Time;

        fn time(&self) -> Self::Time {
            Time(0)
        }
    }
    impl Input for TestInput {
        fn event_id(&self) -> u128 {
            self.0
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(self.0 % 2);
        }
    }

    fn batch(count: u128) -> RouterMessage<TestInput, ()> {
        let events = prepare_inputs((0..count).map(TestInput).collect());
        let (completion, _rejections) = unbounded::<RejectionMessage<RejectionReason>>();
        RouterMessage::Apply(<RouterBatch<TestInput> as ApplyOutput<_, _>>::create(events, completion))
    }

    #[test]
    fn router_process_forwards_every_snapshot_route_to_workers() {
        let (input, receiver) = unbounded();
        let (worker, output) = unbounded::<WorkerMessage<TestInput, ()>>();
        input.send(batch(4)).unwrap();
        drop(input);

        RuntimeRouter::run(RouterProcess::new(crate::contime_router::Placement::default()), receiver, vec![worker]).unwrap();

        let WorkerMessage::Apply(routed) = output.recv().unwrap() else { panic!("expected apply") };
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
                    RuntimeRouter::run(RouterProcess::new(crate::contime_router::Placement::default()), receiver, vec![worker]).unwrap();
                    black_box(output.recv().unwrap())
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
