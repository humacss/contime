use std::marker::PhantomData;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use contime_memory::{ConservativeTrackedSize, TrackedArc};
use crossbeam_channel::Sender;

/// Process-wide conservative memory accounting shared by every core adapter.
#[derive(Clone)]
pub struct MemoryBudget {
    pub(crate) state: Arc<MemoryState>,
}

pub(crate) struct MemoryState {
    pub(crate) used: AtomicUsize,
    pub(crate) maximum: usize,
    pub(crate) buffer: usize,
}

/// The event information required by the complete apply pipeline.
pub trait Input: ConservativeTrackedSize + Send + Sync + 'static {
    type Time: contime_worker::AdvanceTime + Send + Sync + 'static;

    fn event_id(&self) -> u128;
    fn time(&self) -> Self::Time;
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}

/// One event whose retained allocation and shared handles are memory tracked.
pub struct TrackedEvent<I>
where
    I: Input,
{
    pub(crate) inner: TrackedArc<I, MemoryBudget>,
}

/// A core-owned reason returned at the public apply boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RejectionReason {
    BeforeHistoryHorizon,
    MemoryFull,
}

/// Request completion forwarded unchanged from API admission to the worker.
#[derive(Clone)]
pub struct CompletionHandle {
    pub(crate) sender: Sender<contime_api::RejectionMessage<RejectionReason>>,
}

pub struct Advance<T> {
    pub(crate) time: T,
    pub(crate) completion: Sender<()>,
}

/// One admitted API batch consumed by a router.
pub struct RouterBatch<I>
where
    I: Input,
{
    pub(crate) inputs: Vec<TrackedEvent<I>>,
    pub(crate) completion: CompletionHandle,
}

pub struct SnapshotQuery<T, S> {
    pub(crate) time: T,
    pub(crate) snapshot_ids: Vec<u128>,
    pub(crate) response: Sender<Vec<Box<S>>>,
}

pub struct EventQuery<T, I>
where
    I: Input,
{
    pub(crate) snapshot_id: u128,
    pub(crate) from: T,
    pub(crate) to: T,
    pub(crate) response: Sender<Vec<TrackedEvent<I>>>,
}

/// Notification emitted by a registered snapshot listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotListenerMessage<T> {
    Registered { time: T, snapshot_ids: Vec<u128> },
    Replayed { time: T, snapshot_ids: Vec<u128> },
}

/// Core adapter around a consumer-owned notification sender.
#[derive(Clone)]
pub struct SnapshotListener<T> {
    pub(crate) notifications: Sender<SnapshotListenerMessage<T>>,
}

/// One snapshot-listener registration routed to owning workers.
pub struct SnapshotListen<T> {
    pub(crate) time: T,
    pub(crate) snapshot_ids: Vec<u128>,
    pub(crate) listener: SnapshotListener<T>,
}

pub enum RouterMessage<I, S>
where
    I: Input,
{
    Apply(RouterBatch<I>),
    SnapshotQuery(SnapshotQuery<I::Time, S>),
    EventQuery(EventQuery<I::Time, I>),
    SnapshotListen(SnapshotListen<I::Time>),
    Advance(Advance<I::Time>),
}

/// One snapshot-specific route emitted by a router.
pub struct Route<I>
where
    I: Input,
{
    pub(crate) snapshot_id: u128,
    pub(crate) input: TrackedEvent<I>,
}

/// One routed apply batch consumed by a worker.
pub struct WorkerBatch<I>
where
    I: Input,
{
    pub(crate) routes: Vec<Route<I>>,
    pub(crate) completion: CompletionHandle,
}

pub enum WorkerMessage<I, S>
where
    I: Input,
{
    Apply(WorkerBatch<I>),
    SnapshotQuery(SnapshotQuery<I::Time, S>),
    EventQuery(EventQuery<I::Time, I>),
    SnapshotListen(SnapshotListen<I::Time>),
    Advance(Advance<I::Time>),
}

pub(crate) struct History<I>
where
    I: Input,
{
    pub(crate) events: contime_events::EventHistory<TrackedEvent<I>>,
}

pub(crate) enum HistoryIter<'a, I>
where
    I: Input,
{
    All(contime_events::EventHistoryIter<'a, TrackedEvent<I>>),
    Range(contime_events::EventHistoryRangeIter<'a, TrackedEvent<I>>),
}

pub(crate) struct CheckpointStorageConfig {
    pub(crate) checkpoints: contime_checkpoints::CheckpointConfig,
    pub(crate) budget: MemoryBudget,
}

pub(crate) struct CheckpointState<S>
where
    S: contime_checkpoints::Snapshot,
{
    pub(crate) checkpoints: contime_checkpoints::CheckpointStore<S>,
}

pub(crate) struct CheckpointStorage<S, W>
where
    S: contime_checkpoints::Snapshot + ConservativeTrackedSize,
{
    pub(crate) state: contime_memory::TrackedBox<CheckpointState<S>, MemoryBudget>,
    pub(crate) wrapper: PhantomData<fn() -> W>,
}

/// Deterministic router execution supplied to the runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouterProcess<I, S>
where
    I: Input,
{
    pub(crate) seed: u64,
    pub(crate) input: PhantomData<fn() -> (I, S)>,
}

/// Replay worker execution supplied to the runtime.
pub struct WorkerProcess<I, S, W>
where
    I: Input,
    S: contime_checkpoints::Snapshot + ConservativeTrackedSize,
{
    pub(crate) worker: contime_worker::WorkerConfig,
    pub(crate) checkpoints: contime_checkpoints::CheckpointConfig,
    pub(crate) history_retention: I::Time,
    pub(crate) budget: MemoryBudget,
    pub(crate) wrapper: W,
    pub(crate) types: PhantomData<fn() -> (I, S)>,
}

/// Complete apply-and-query process configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConTimeConfig<T> {
    pub router_count: usize,
    pub worker_count: usize,
    pub router_seed: u64,
    pub memory_limit: usize,
    pub memory_buffer: usize,
    pub history_retention: T,
    pub worker: contime_worker::WorkerConfig,
    pub checkpoints: contime_checkpoints::CheckpointConfig,
}

/// A running, memory-accounted apply-and-query pipeline.
pub struct ConTime<I, S, W>
where
    I: Input,
{
    pub(crate) runtime: contime_runtime::Runtime<RouterMessage<I, S>, contime_router::RouterError, std::convert::Infallible>,
    pub(crate) budget: MemoryBudget,
    pub(crate) types: PhantomData<fn() -> (S, W)>,
}
