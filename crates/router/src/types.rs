#[derive(Debug)]
pub struct InputBatch<I, C> {
    pub inputs: Vec<I>,
    pub completion: C,
}

/// A caller-selected batch consumed by the router.
pub trait RouteInputBatch {
    type Input;
    type Completion;

    fn into_parts(self) -> (Vec<Self::Input>, Self::Completion);
}

impl<I, C> RouteInputBatch for InputBatch<I, C> {
    type Input = I;
    type Completion = C;

    fn into_parts(self) -> (Vec<Self::Input>, Self::Completion) {
        (self.inputs, self.completion)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RoutedInput<I> {
    pub snapshot_id: u128,
    pub input: I,
}

/// Constructs one caller-selected route emitted by the router.
pub trait RouteOutput<I>: Sized {
    fn create(snapshot_id: u128, input: I) -> Self;
}

impl<I> RouteOutput<I> for RoutedInput<I> {
    fn create(snapshot_id: u128, input: I) -> Self {
        Self { snapshot_id, input }
    }
}

#[derive(Debug)]
pub struct WorkerBatch<I, C> {
    pub inputs: Vec<RoutedInput<I>>,
    pub completion: C,
}

/// Constructs one caller-selected worker message emitted by the router.
pub trait WorkerOutput<I, C>: Sized {
    type Route: RouteOutput<I>;

    fn create(inputs: Vec<Self::Route>, completion: C) -> Self;
}

impl<I, C> WorkerOutput<I, C> for WorkerBatch<I, C> {
    type Route = RoutedInput<I>;

    fn create(inputs: Vec<Self::Route>, completion: C) -> Self {
        Self { inputs, completion }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouterError {
    NoWorkers,
    WorkerUnavailable { worker_index: usize },
}

pub trait RoutableInput {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}

/// Caller-selected historical snapshot query consumed by the router.
pub trait SnapshotQueryInput {
    type Time;
    type Response: Clone;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response);
}

/// Caller-selected event-history query consumed by the router.
pub trait EventQueryInput {
    type Time;
    type Response;

    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response);
}

/// Constructs a caller-selected worker snapshot-query message.
pub trait SnapshotQueryWorkerOutput<T, R>: Sized {
    fn snapshot_query(time: T, snapshot_ids: Vec<u128>, response: R) -> Self;
}

/// Constructs a caller-selected worker event-query message.
pub trait EventQueryWorkerOutput<T, R>: Sized {
    fn event_query(snapshot_id: u128, from: T, to: T, response: R) -> Self;
}

/// One of the operations accepted by a unified router queue.
pub enum RouteInputKind<A, SQ, EQ> {
    Apply(A),
    SnapshotQuery(SQ),
    EventQuery(EQ),
}

/// Converts a caller-selected router message into its static operation kind.
pub trait RouteInput {
    type Apply;
    type SnapshotQuery;
    type EventQuery;

    fn into_kind(self) -> RouteInputKind<Self::Apply, Self::SnapshotQuery, Self::EventQuery>;
}
