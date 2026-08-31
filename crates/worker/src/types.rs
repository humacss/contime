use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// Worker-local replay scheduling policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerConfig {
    /// Maximum time changed events may wait before their snapshot is replayed.
    pub maximum_dirty_age: Duration,
    /// Maximum non-overdue snapshots replayed after each received batch.
    pub replays_per_receive: usize,
    /// Deadline entries tolerated before relative compaction is considered.
    pub deadline_compaction_minimum: usize,
    /// Maximum deadline entries per active snapshot before compaction.
    pub deadline_compaction_multiplier: usize,
}

/// One event route already assigned to this worker.
#[derive(Debug)]
pub struct RoutedInput<I> {
    pub snapshot_id: u128,
    pub input: I,
}

/// A caller-selected route consumed by a worker.
pub trait RouteInput {
    type Input;

    fn into_parts(self) -> (u128, Self::Input);
}

impl<I> RouteInput for RoutedInput<I> {
    type Input = I;

    fn into_parts(self) -> (u128, Self::Input) {
        (self.snapshot_id, self.input)
    }
}

/// One worker apply message.
#[derive(Debug)]
pub struct ApplyBatch<I, C> {
    pub inputs: Vec<RoutedInput<I>>,
    pub completion: C,
}

/// A caller-selected apply message consumed by a worker.
pub trait ApplyInput {
    type Route: RouteInput;
    type Completion;

    fn into_parts(self) -> (Vec<Self::Route>, Self::Completion);
}

impl<I, C> ApplyInput for ApplyBatch<I, C> {
    type Route = RoutedInput<I>;
    type Completion = C;

    fn into_parts(self) -> (Vec<Self::Route>, Self::Completion) {
        (self.inputs, self.completion)
    }
}

/// A request-scoped response handle forwarded opaquely through the router.
///
/// Successful completion is represented by dropping the handle without
/// calling `reject`.
pub trait Completion<R> {
    fn reject(self, rejections: Vec<R>);
}

impl<R> Completion<R> for crossbeam_channel::Sender<Vec<R>> {
    fn reject(self, rejections: Vec<R>) {
        let _ = self.send(rejections);
    }
}

/// The result of inserting one input into the canonical event store.
#[derive(Debug)]
pub struct EventInsert<R> {
    pub changed: bool,
    pub rejections: Vec<R>,
}

/// Canonical per-snapshot event storage owned by a worker.
pub trait Events<I>: Sized {
    type Config;
    type Rejection;

    fn create(snapshot_id: u128, config: &Self::Config) -> Self;

    fn insert(&mut self, input: I) -> EventInsert<Self::Rejection>;
}

/// Read-only event-history access required by worker queries.
pub trait QueryEvents<I> {
    type Time: Clone + Ord;

    fn clone_between(&self, from: &Self::Time, to: &Self::Time) -> Vec<I>
    where
        I: Clone;
}

/// Materialized per-snapshot checkpoint storage.
///
/// Checkpoint updates can inspect canonical events but cannot modify them.
pub trait Checkpoints<S>: Sized {
    type Config;
    type Context;

    fn create(snapshot_id: u128, config: &Self::Config) -> Self;

    fn update(&mut self, events: &mut S, context: &mut Self::Context);
}

/// Read-only checkpoint reconstruction required by worker queries.
pub trait QueryCheckpoints<E> {
    type Context;
    type Time: Clone + Ord;
    type Snapshot;

    fn query_at(&self, events: &E, context: &mut Self::Context, time: Self::Time) -> Option<Box<Self::Snapshot>>;
}

pub trait SnapshotQueryInput {
    type Time;
    type Response;

    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response);
}

pub trait EventQueryInput {
    type Time;
    type Response;

    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response);
}

pub trait SnapshotQueryResponse<S> {
    fn send(self, snapshots: Vec<Box<S>>);
}

impl<S> SnapshotQueryResponse<S> for crossbeam_channel::Sender<Vec<Box<S>>> {
    fn send(self, snapshots: Vec<Box<S>>) {
        let _ = crossbeam_channel::Sender::send(&self, snapshots);
    }
}

pub trait EventQueryResponse<I> {
    fn send(self, events: Vec<I>);
}

impl<I> EventQueryResponse<I> for crossbeam_channel::Sender<Vec<I>> {
    fn send(self, events: Vec<I>) {
        let _ = crossbeam_channel::Sender::send(&self, events);
    }
}

pub enum WorkInputKind<A, SQ, EQ> {
    Apply(A),
    SnapshotQuery(SQ),
    EventQuery(EQ),
}

pub trait WorkInput {
    type Apply;
    type SnapshotQuery;
    type EventQuery;

    fn into_kind(self) -> WorkInputKind<Self::Apply, Self::SnapshotQuery, Self::EventQuery>;
}

pub(crate) type Request<C, R> = Rc<RefCell<PendingRequest<C, R>>>;

pub(crate) struct SnapshotSlot<S, K, C, R> {
    pub(crate) events: S,
    pub(crate) checkpoints: Option<K>,
    pub(crate) waiters: Vec<Request<C, R>>,
}

pub(crate) struct PendingRequest<C, R> {
    pub(crate) completion: Option<C>,
    pub(crate) pending_snapshots: usize,
    pub(crate) rejections: Vec<R>,
}

pub(crate) fn new_request<C, R>(completion: C) -> Request<C, R> {
    Rc::new(RefCell::new(PendingRequest { completion: Some(completion), pending_snapshots: 0, rejections: Vec::new() }))
}

pub(crate) fn register_waiter<S, K, C, R>(slot: &mut SnapshotSlot<S, K, C, R>, request: &Request<C, R>) {
    if slot.waiters.iter().any(|waiter| Rc::ptr_eq(waiter, request)) {
        return;
    }

    request.borrow_mut().pending_snapshots += 1;
    slot.waiters.push(Rc::clone(request));
}

pub(crate) fn complete_snapshot<C, R>(request: Request<C, R>)
where
    C: Completion<R>,
{
    request.borrow_mut().pending_snapshots -= 1;
    finish_if_ready(&request);
}

pub(crate) fn finish_if_ready<C, R>(request: &Request<C, R>)
where
    C: Completion<R>,
{
    let completed = {
        let mut request = request.borrow_mut();
        if request.pending_snapshots != 0 {
            None
        } else {
            request.completion.take().map(|completion| {
                let rejections = std::mem::take(&mut request.rejections);
                (completion, rejections)
            })
        }
    };

    if let Some((completion, rejections)) = completed {
        if rejections.is_empty() {
            drop(completion);
        } else {
            completion.reject(rejections);
        }
    }
}
