use std::convert::Infallible;

use contime_api::{ApplyOutput, RejectionMessage};
use contime_router::{RouteInputBatch, RouteOutput, WorkerOutput};
use contime_worker::{ApplyInput, Completion, RouteInput};
use crossbeam_channel::Sender;

use crate::{CompletionHandle, Input, RejectionReason, Route, RouterBatch, TrackedEvent, WorkerBatch};

impl CompletionHandle {
    pub fn new(sender: Sender<RejectionMessage<RejectionReason>>) -> Self {
        Self { _sender: sender }
    }
}

impl Completion<Infallible> for CompletionHandle {
    fn reject(self, rejections: Vec<Infallible>) {
        for rejection in rejections {
            match rejection {}
        }
        drop(self);
    }
}

impl<I> ApplyOutput<TrackedEvent<I>, RejectionReason> for RouterBatch<I>
where
    I: Input,
{
    fn create(inputs: Vec<TrackedEvent<I>>, rejection_sender: Sender<RejectionMessage<RejectionReason>>) -> Self {
        Self { inputs, completion: CompletionHandle::new(rejection_sender) }
    }
}

impl<I> RouteInputBatch for RouterBatch<I>
where
    I: Input,
{
    type Input = TrackedEvent<I>;
    type Completion = CompletionHandle;

    fn into_parts(self) -> (Vec<Self::Input>, Self::Completion) {
        (self.inputs, self.completion)
    }
}

impl<I> RouteOutput<TrackedEvent<I>> for Route<I>
where
    I: Input,
{
    fn create(snapshot_id: u128, input: TrackedEvent<I>) -> Self {
        Self { snapshot_id, input }
    }
}

impl<I> RouteInput for Route<I>
where
    I: Input,
{
    type Input = TrackedEvent<I>;

    fn into_parts(self) -> (u128, Self::Input) {
        (self.snapshot_id, self.input)
    }
}

impl<I> WorkerOutput<TrackedEvent<I>, CompletionHandle> for WorkerBatch<I>
where
    I: Input,
{
    type Route = Route<I>;

    fn create(inputs: Vec<Self::Route>, completion: CompletionHandle) -> Self {
        Self { routes: inputs, completion }
    }
}

impl<I> ApplyInput for WorkerBatch<I>
where
    I: Input,
{
    type Route = Route<I>;
    type Completion = CompletionHandle;

    fn into_parts(self) -> (Vec<Self::Route>, Self::Completion) {
        (self.routes, self.completion)
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::hint::black_box;

    use contime_api::{ApplyOutput, RejectionMessage};
    use contime_memory::ConservativeTrackedSize;
    use contime_router::{RouteInputBatch, RouteOutput, WorkerOutput};
    use contime_worker::{ApplyInput, Completion, RouteInput};
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, TryRecvError};

    use crate::input::prepare_inputs;
    use crate::{CompletionHandle, Input, MemoryBudget, RejectionReason, Route, RouterBatch, WorkerBatch};

    #[derive(Debug)]
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
            emit(7);
        }
    }

    #[test]
    fn adjacent_adapters_preserve_the_tracked_event_allocation() {
        let budget = MemoryBudget::new(10_000, 100);
        let mut events = prepare_inputs(&budget, vec![TestInput(9)]).unwrap();
        let expected = events[0].as_ref() as *const TestInput;
        let (rejections, _receiver) = unbounded::<RejectionMessage<RejectionReason>>();
        let batch = <RouterBatch<TestInput> as ApplyOutput<_, _>>::create(events.drain(..).collect(), rejections);
        let (events, completion) = batch.into_parts();
        let route = <Route<TestInput> as RouteOutput<_>>::create(7, events.into_iter().next().unwrap());
        let worker = <WorkerBatch<TestInput> as WorkerOutput<_, _>>::create(vec![route], completion);
        let (routes, _completion) = worker.into_parts();
        let (snapshot_id, event) = routes.into_iter().next().unwrap().into_parts();

        assert_eq!(snapshot_id, 7);
        assert_eq!(event.as_ref() as *const TestInput, expected);
    }

    #[test]
    fn dropping_successful_completion_closes_the_api_rejection_channel() {
        let (sender, receiver) = unbounded::<RejectionMessage<RejectionReason>>();
        let completion = CompletionHandle::new(sender);

        <CompletionHandle as Completion<Infallible>>::reject(completion, Vec::new());

        assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_message() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/message/1000_routes", |bencher| {
            bencher.iter_batched(
                || {
                    let budget = MemoryBudget::new(usize::MAX, 0);
                    let events = prepare_inputs(&budget, (0..1_000).map(TestInput).collect()).unwrap();
                    let (sender, _receiver) = unbounded();
                    (events, CompletionHandle::new(sender))
                },
                |(events, completion)| {
                    let routes = events.into_iter().map(|event| <Route<TestInput> as RouteOutput<_>>::create(7, event)).collect();
                    black_box(<WorkerBatch<TestInput> as WorkerOutput<_, _>>::create(routes, completion))
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
