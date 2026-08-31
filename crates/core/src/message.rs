use std::convert::Infallible;

use contime_api::{ApplyOutput, EventQueryOutput, RejectionMessage, SnapshotQueryOutput};
use contime_router::{
    EventQueryInput as RouterEventQueryInput, EventQueryWorkerOutput, RouteInput as RouterInput, RouteInputBatch, RouteInputKind,
    RouteOutput, SnapshotQueryInput as RouterSnapshotQueryInput, SnapshotQueryWorkerOutput, WorkerOutput,
};
use contime_worker::{
    ApplyInput, Completion, EventQueryInput as WorkerEventQueryInput, RouteInput, SnapshotQueryInput as WorkerSnapshotQueryInput,
    WorkInput, WorkInputKind,
};
use crossbeam_channel::Sender;

use crate::{
    CompletionHandle, EventQuery, Input, RejectionReason, Route, RouterBatch, RouterMessage, SnapshotQuery, TrackedEvent, WorkerBatch,
    WorkerMessage,
};

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

impl<I, S> ApplyOutput<TrackedEvent<I>, RejectionReason> for RouterMessage<I, S>
where
    I: Input,
{
    fn create(inputs: Vec<TrackedEvent<I>>, rejection_sender: Sender<RejectionMessage<RejectionReason>>) -> Self {
        Self::Apply(RouterBatch { inputs, completion: CompletionHandle::new(rejection_sender) })
    }
}

impl<I, S> SnapshotQueryOutput<I::Time, S> for RouterMessage<I, S>
where
    I: Input,
{
    fn snapshot_query(time: I::Time, snapshot_ids: Vec<u128>, response: Sender<Vec<Box<S>>>) -> Self {
        Self::SnapshotQuery(SnapshotQuery { time, snapshot_ids, response })
    }
}

impl<I, S> EventQueryOutput<I::Time, TrackedEvent<I>> for RouterMessage<I, S>
where
    I: Input,
{
    fn event_query(snapshot_id: u128, from: I::Time, to: I::Time, response: Sender<Vec<TrackedEvent<I>>>) -> Self {
        Self::EventQuery(EventQuery { snapshot_id, from, to, response })
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

impl<T, S> RouterSnapshotQueryInput for SnapshotQuery<T, S> {
    type Time = T;
    type Response = Sender<Vec<Box<S>>>;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response) {
        (self.time, self.snapshot_ids, self.response)
    }
}

impl<T, I> RouterEventQueryInput for EventQuery<T, I>
where
    I: Input,
{
    type Time = T;
    type Response = Sender<Vec<TrackedEvent<I>>>;

    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response) {
        (self.snapshot_id, self.from, self.to, self.response)
    }
}

impl<I, S> RouterInput for RouterMessage<I, S>
where
    I: Input,
{
    type Apply = RouterBatch<I>;
    type SnapshotQuery = SnapshotQuery<I::Time, S>;
    type EventQuery = EventQuery<I::Time, I>;

    fn into_kind(self) -> RouteInputKind<RouterBatch<I>, SnapshotQuery<I::Time, S>, EventQuery<I::Time, I>> {
        match self {
            Self::Apply(batch) => RouteInputKind::Apply(batch),
            Self::SnapshotQuery(query) => RouteInputKind::SnapshotQuery(query),
            Self::EventQuery(query) => RouteInputKind::EventQuery(query),
        }
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

impl<I, S> WorkerOutput<TrackedEvent<I>, CompletionHandle> for WorkerMessage<I, S>
where
    I: Input,
{
    type Route = Route<I>;

    fn create(inputs: Vec<Self::Route>, completion: CompletionHandle) -> Self {
        Self::Apply(WorkerBatch { routes: inputs, completion })
    }
}

impl<I, S> SnapshotQueryWorkerOutput<I::Time, Sender<Vec<Box<S>>>> for WorkerMessage<I, S>
where
    I: Input,
{
    fn snapshot_query(time: I::Time, snapshot_ids: Vec<u128>, response: Sender<Vec<Box<S>>>) -> Self {
        Self::SnapshotQuery(SnapshotQuery { time, snapshot_ids, response })
    }
}

impl<I, S> EventQueryWorkerOutput<I::Time, Sender<Vec<TrackedEvent<I>>>> for WorkerMessage<I, S>
where
    I: Input,
{
    fn event_query(snapshot_id: u128, from: I::Time, to: I::Time, response: Sender<Vec<TrackedEvent<I>>>) -> Self {
        Self::EventQuery(EventQuery { snapshot_id, from, to, response })
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

impl<T, S> WorkerSnapshotQueryInput for SnapshotQuery<T, S> {
    type Time = T;
    type Response = Sender<Vec<Box<S>>>;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response) {
        (self.time, self.snapshot_ids, self.response)
    }
}

impl<T, I> WorkerEventQueryInput for EventQuery<T, I>
where
    I: Input,
{
    type Time = T;
    type Response = Sender<Vec<TrackedEvent<I>>>;

    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response) {
        (self.snapshot_id, self.from, self.to, self.response)
    }
}

impl<I, S> WorkInput for WorkerMessage<I, S>
where
    I: Input,
{
    type Apply = WorkerBatch<I>;
    type SnapshotQuery = SnapshotQuery<I::Time, S>;
    type EventQuery = EventQuery<I::Time, I>;

    fn into_kind(self) -> WorkInputKind<WorkerBatch<I>, SnapshotQuery<I::Time, S>, EventQuery<I::Time, I>> {
        match self {
            Self::Apply(batch) => WorkInputKind::Apply(batch),
            Self::SnapshotQuery(query) => WorkInputKind::SnapshotQuery(query),
            Self::EventQuery(query) => WorkInputKind::EventQuery(query),
        }
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
