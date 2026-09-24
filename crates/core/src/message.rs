use contime_api::{
    AdvanceOutput as ApiAdvanceOutput, ApplyOutput, EventQueryOutput, RejectionMessage, SnapshotListenOutput as ApiSnapshotListenOutput,
    SnapshotQueryOutput,
};
use contime_router::{
    AdvanceInput as RouterAdvanceInput, AdvanceWorkerOutput, EventQueryInput as RouterEventQueryInput, EventQueryWorkerOutput,
    RouteInput as RouterInput, RouteInputBatch, RouteInputKind, RouteOutput, SnapshotListenInput as RouterSnapshotListenInput,
    SnapshotListenWorkerOutput, SnapshotQueryInput as RouterSnapshotQueryInput, SnapshotQueryWorkerOutput, WorkerOutput,
};
use contime_worker::{
    AdvanceInput as WorkerAdvanceInput, ApplyInput, Completion, EventQueryInput as WorkerEventQueryInput, RouteInput,
    SnapshotListenInput as WorkerSnapshotListenInput, SnapshotListener as WorkerSnapshotListener,
    SnapshotQueryInput as WorkerSnapshotQueryInput, WorkInput, WorkInputKind,
};
use crossbeam_channel::Sender;

use crate::{
    Advance, CompletionHandle, EventQuery, Input, RejectionReason, Route, RouterBatch, RouterMessage, SharedEvent, SnapshotListen,
    SnapshotListener, SnapshotListenerMessage, SnapshotQuery, WorkerBatch, WorkerMessage,
};

impl<T> SnapshotListener<T> {
    pub(crate) fn new(notifications: Sender<SnapshotListenerMessage<T>>) -> Self {
        Self { notifications }
    }
}

impl<T> WorkerSnapshotListener<T> for SnapshotListener<T>
where
    T: Clone,
{
    fn registered(&self, time: T, snapshot_ids: Vec<u128>) -> bool {
        self.notifications.send(SnapshotListenerMessage::Registered { time, snapshot_ids }).is_ok()
    }

    fn replayed(&self, time: T, snapshot_ids: Vec<u128>) -> bool {
        self.notifications.send(SnapshotListenerMessage::Replayed { time, snapshot_ids }).is_ok()
    }
}

impl CompletionHandle {
    pub fn new(sender: Sender<RejectionMessage<RejectionReason>>) -> Self {
        Self { sender }
    }
}

impl Completion<RejectionMessage<RejectionReason>> for CompletionHandle {
    fn reject(self, rejections: Vec<RejectionMessage<RejectionReason>>) {
        for rejection in rejections {
            let _ = self.sender.send(rejection);
        }
    }
}

impl<I, S> ApiAdvanceOutput<I::Time> for RouterMessage<I, S>
where
    I: Input,
{
    fn advance(time: I::Time, completion: Sender<()>) -> Self {
        Self::Advance(Advance { time, completion })
    }
}

impl<T> RouterAdvanceInput for Advance<T>
where
    T: Clone,
{
    type Time = T;
    type Completion = Sender<()>;

    fn into_parts(self) -> (T, Sender<()>) {
        (self.time, self.completion)
    }
}

impl<I> ApplyOutput<SharedEvent<I>, RejectionReason> for RouterBatch<I>
where
    I: Input,
{
    fn create(inputs: Vec<SharedEvent<I>>, rejection_sender: Sender<RejectionMessage<RejectionReason>>) -> Self {
        Self { inputs, completion: CompletionHandle::new(rejection_sender) }
    }
}

impl<I, S> ApplyOutput<SharedEvent<I>, RejectionReason> for RouterMessage<I, S>
where
    I: Input,
{
    fn create(inputs: Vec<SharedEvent<I>>, rejection_sender: Sender<RejectionMessage<RejectionReason>>) -> Self {
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

impl<I, S> EventQueryOutput<I::Time, SharedEvent<I>> for RouterMessage<I, S>
where
    I: Input,
{
    fn event_query(snapshot_id: u128, from: I::Time, to: I::Time, response: Sender<Vec<SharedEvent<I>>>) -> Self {
        Self::EventQuery(EventQuery { snapshot_id, from, to, response })
    }
}

impl<I, S> ApiSnapshotListenOutput<I::Time, SnapshotListenerMessage<I::Time>> for RouterMessage<I, S>
where
    I: Input,
{
    fn listen(time: I::Time, snapshot_ids: Vec<u128>, notifications: Sender<SnapshotListenerMessage<I::Time>>) -> Self {
        Self::SnapshotListen(SnapshotListen { time, snapshot_ids, listener: SnapshotListener::new(notifications) })
    }
}

impl<I> RouteInputBatch for RouterBatch<I>
where
    I: Input,
{
    type Input = SharedEvent<I>;
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
    type Response = Sender<Vec<SharedEvent<I>>>;

    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response) {
        (self.snapshot_id, self.from, self.to, self.response)
    }
}

impl<T> RouterSnapshotListenInput for SnapshotListen<T>
where
    T: Clone,
{
    type Time = T;
    type Listener = SnapshotListener<T>;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener) {
        (self.time, self.snapshot_ids, self.listener)
    }
}

impl<I, S> RouterInput for RouterMessage<I, S>
where
    I: Input,
{
    type Apply = RouterBatch<I>;
    type SnapshotQuery = SnapshotQuery<I::Time, S>;
    type EventQuery = EventQuery<I::Time, I>;
    type SnapshotListen = SnapshotListen<I::Time>;
    type Advance = Advance<I::Time>;

    fn into_kind(
        self,
    ) -> RouteInputKind<RouterBatch<I>, SnapshotQuery<I::Time, S>, EventQuery<I::Time, I>, SnapshotListen<I::Time>, Advance<I::Time>> {
        match self {
            Self::Apply(batch) => RouteInputKind::Apply(batch),
            Self::SnapshotQuery(query) => RouteInputKind::SnapshotQuery(query),
            Self::EventQuery(query) => RouteInputKind::EventQuery(query),
            Self::SnapshotListen(registration) => RouteInputKind::SnapshotListen(registration),
            Self::Advance(advance) => RouteInputKind::Advance(advance),
            Self::Prune(prune) => RouteInputKind::Prune(prune),
            Self::Resolve { round, prune } => RouteInputKind::Resolve { round, prune },
            Self::Fence { round, observed } => RouteInputKind::Fence { round, observed },
            Self::Internal { .. } | Self::Report { .. } | Self::Pruned { .. } | Self::SubscribePrunedHorizon(_) | Self::Shutdown => {
                unreachable!("coordinator-only message reached router")
            }
        }
    }
}

impl<I> RouteOutput<SharedEvent<I>> for Route<I>
where
    I: Input,
{
    fn create(snapshot_id: u128, input: SharedEvent<I>) -> Self {
        Self { snapshot_id, input }
    }
}

impl<I> RouteInput for Route<I>
where
    I: Input,
{
    type Input = SharedEvent<I>;

    fn into_parts(self) -> (u128, Self::Input) {
        (self.snapshot_id, self.input)
    }
}

impl<I> WorkerOutput<SharedEvent<I>, CompletionHandle> for WorkerBatch<I>
where
    I: Input,
{
    type Route = Route<I>;

    fn create(inputs: Vec<Self::Route>, completion: CompletionHandle) -> Self {
        Self { routes: inputs, completion }
    }
}

impl<I, S> WorkerOutput<SharedEvent<I>, CompletionHandle> for WorkerMessage<I, S>
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

impl<I, S> EventQueryWorkerOutput<I::Time, Sender<Vec<SharedEvent<I>>>> for WorkerMessage<I, S>
where
    I: Input,
{
    fn event_query(snapshot_id: u128, from: I::Time, to: I::Time, response: Sender<Vec<SharedEvent<I>>>) -> Self {
        Self::EventQuery(EventQuery { snapshot_id, from, to, response })
    }
}

impl<I, S> SnapshotListenWorkerOutput<I::Time, SnapshotListener<I::Time>> for WorkerMessage<I, S>
where
    I: Input,
{
    fn listen(time: I::Time, snapshot_ids: Vec<u128>, listener: SnapshotListener<I::Time>) -> Self {
        Self::SnapshotListen(SnapshotListen { time, snapshot_ids, listener })
    }
}

impl<I, S> AdvanceWorkerOutput<I::Time, Sender<()>> for WorkerMessage<I, S>
where
    I: Input,
{
    fn advance(time: I::Time, completion: Sender<()>) -> Self {
        Self::Advance(Advance { time, completion })
    }
}

impl<T> WorkerAdvanceInput for Advance<T>
where
    T: contime_worker::AdvanceTime,
{
    type Time = T;
    type Completion = Sender<()>;

    fn into_parts(self) -> (T, Sender<()>) {
        (self.time, self.completion)
    }
}

impl<I: Input, S> contime_router::CoordinationOutput<I::Time, Sender<()>> for WorkerMessage<I, S> {
    fn resolve(round: u64, time: I::Time, completion: Sender<()>) -> Self {
        Self::Resolve { round, prune: Advance { time, completion } }
    }
    fn fence(round: u64, router: usize) -> Self {
        Self::Fence { round, router }
    }
    fn prune(time: I::Time, completion: Sender<()>) -> Self {
        Self::Prune(Advance { time, completion })
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
    type Response = Sender<Vec<SharedEvent<I>>>;

    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response) {
        (self.snapshot_id, self.from, self.to, self.response)
    }
}

impl<T> WorkerSnapshotListenInput for SnapshotListen<T>
where
    T: Clone + Ord,
{
    type Time = T;
    type Listener = SnapshotListener<T>;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener) {
        (self.time, self.snapshot_ids, self.listener)
    }
}

impl<I, S> WorkInput for WorkerMessage<I, S>
where
    I: Input,
{
    type Apply = WorkerBatch<I>;
    type SnapshotQuery = SnapshotQuery<I::Time, S>;
    type EventQuery = EventQuery<I::Time, I>;
    type SnapshotListen = SnapshotListen<I::Time>;
    type Advance = Advance<I::Time>;

    fn into_kind(
        self,
    ) -> WorkInputKind<WorkerBatch<I>, SnapshotQuery<I::Time, S>, EventQuery<I::Time, I>, SnapshotListen<I::Time>, Advance<I::Time>> {
        match self {
            Self::Apply(batch) => WorkInputKind::Apply(batch),
            Self::SnapshotQuery(query) => WorkInputKind::SnapshotQuery(query),
            Self::EventQuery(query) => WorkInputKind::EventQuery(query),
            Self::SnapshotListen(registration) => WorkInputKind::SnapshotListen(registration),
            Self::Advance(advance) => WorkInputKind::Advance(advance),
            Self::Prune(prune) => WorkInputKind::Prune(prune),
            Self::Resolve { round, prune } => WorkInputKind::Resolve { round, prune },
            Self::Fence { round, router } => WorkInputKind::Fence { round, router },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::types::testing::Time;
    use std::hint::black_box;

    use contime_api::{AdvanceOutput, ApplyOutput, RejectionMessage};

    use contime_router::{AdvanceInput as RouterAdvanceInput, AdvanceWorkerOutput, RouteInputBatch, RouteOutput, WorkerOutput};
    use contime_worker::{AdvanceInput as WorkerAdvanceInput, ApplyInput, Completion, RouteInput};
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, TryRecvError};

    use crate::input::prepare_inputs;
    use crate::{CompletionHandle, Input, RejectionReason, Route, RouterBatch, RouterMessage, WorkerBatch, WorkerMessage};

    #[derive(Debug)]
    struct TestInput(u128);

    impl contime_checkpoints::Event for TestInput {
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
            emit(7);
        }
    }

    #[test]
    fn adjacent_adapters_share_the_event_allocation() {
        let mut events = prepare_inputs(vec![TestInput(9)]);
        let expected = events[0].as_ref() as *const TestInput;
        let (rejections, _receiver) = unbounded::<RejectionMessage<RejectionReason>>();
        let batch = <RouterBatch<TestInput> as ApplyOutput<_, _>>::create(std::mem::take(&mut events), rejections);
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

        <CompletionHandle as Completion<RejectionMessage<RejectionReason>>>::reject(completion, Vec::new());

        assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn advance_adapters_preserve_the_timestamp_and_completion_channel() {
        let (completion, done) = unbounded();
        let router = <RouterMessage<TestInput, ()> as AdvanceOutput<Time>>::advance(Time(50), completion);
        let RouterMessage::Advance(advance) = router else { panic!("expected router advance") };
        let (time, completion) = RouterAdvanceInput::into_parts(advance);
        let worker = <WorkerMessage<TestInput, ()> as AdvanceWorkerOutput<_, _>>::advance(time, completion);
        let WorkerMessage::Advance(advance) = worker else { panic!("expected worker advance") };
        let (time, completion) = WorkerAdvanceInput::into_parts(advance);

        assert_eq!(time, Time(50));
        drop(completion);
        assert_eq!(done.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_message() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/message/1000_routes", |bencher| {
            bencher.iter_batched(
                || {
                    let events = prepare_inputs((0..1_000).map(TestInput).collect());
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
