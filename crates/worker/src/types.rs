use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// Worker-local scheduling and memory policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerConfig {
    /// Maximum retained bytes owned by this worker's event and checkpoint stores.
    pub memory_limit: u64,
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

/// One independently admitted worker message.
#[derive(Debug)]
pub struct ApplyBatch<I, C> {
    pub inputs: Vec<RoutedInput<I>>,
    pub completion: C,
}

/// The event information required for worker-local memory admission.
pub trait WorkerInput {
    fn input_id(&self) -> u128;
    fn conservative_size(&self) -> u64;
}

/// A rejection owned either by worker admission or by the event store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerRejection<R> {
    MemoryFull { input_id: u128 },
    Event(R),
}

/// A request-scoped response handle forwarded opaquely through the router.
///
/// Successful completion is represented by dropping the handle without
/// calling `reject`.
pub trait Completion<R> {
    fn reject(self, rejections: Vec<WorkerRejection<R>>);
}

impl<R> Completion<R> for crossbeam_channel::Sender<Vec<WorkerRejection<R>>> {
    fn reject(self, rejections: Vec<WorkerRejection<R>>) {
        let _ = self.send(rejections);
    }
}

/// A newly initialized event store and its retained-memory change.
#[derive(Debug)]
pub struct EventsCreated<S> {
    pub events: S,
    pub retained_bytes_delta: i64,
}

/// The result of inserting one input into the canonical event store.
#[derive(Debug)]
pub struct EventInsert<R> {
    pub retained_bytes_delta: i64,
    pub changed: bool,
    pub rejections: Vec<R>,
}

/// Canonical per-snapshot event storage owned by a worker.
pub trait Events<I>: Sized {
    type Config;
    type Rejection;

    fn create(snapshot_id: u128, config: &Self::Config, retained_bytes_limit: u64) -> Option<EventsCreated<Self>>;

    fn insert(&mut self, input: I, retained_bytes_limit: u64) -> EventInsert<Self::Rejection>;
}

/// A newly initialized checkpoint store and its retained-memory change.
#[derive(Debug)]
pub struct CheckpointsCreated<S> {
    pub checkpoints: S,
    pub retained_bytes_delta: i64,
}

/// Materialized per-snapshot checkpoint storage.
///
/// Checkpoint updates can inspect canonical events but cannot modify them.
pub trait Checkpoints<S>: Sized {
    type Config;
    type Context;

    fn create(snapshot_id: u128, config: &Self::Config, retained_bytes_limit: u64) -> CheckpointsCreated<Self>;

    fn update(&mut self, events: &S, context: &mut Self::Context, retained_bytes_limit: u64) -> CheckpointResult;
}

/// The retained-memory change produced by one checkpoint update.
#[derive(Debug)]
pub struct CheckpointResult {
    pub retained_bytes_delta: i64,
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
    pub(crate) rejections: Vec<WorkerRejection<R>>,
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
