use std::marker::PhantomData;

#[cfg(test)]
pub(crate) mod testing {
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
    pub struct Time(pub i64);

    impl contime_checkpoints::Timestamp for Time {
        fn previous(&self) -> Option<Self> {
            self.0.checked_sub(1).map(Self)
        }
    }
    impl contime_worker::AdvanceTime for Time {
        fn saturating_sub(&self, other: &Self) -> Self {
            Self(self.0.saturating_sub(other.0))
        }
    }
}
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};

/// The event information required by the complete apply pipeline.
pub trait Input:
    contime_checkpoints::Event<Time: contime_worker::AdvanceTime + contime_checkpoints::Timestamp + Send + Sync + 'static>
    + Send
    + Sync
    + 'static
{
    fn event_id(&self) -> u128;
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}

/// An immutable event shared by its routed snapshot histories.
pub struct SharedEvent<I>
where
    I: Input,
{
    pub(crate) inner: Arc<I>,
}

/// A core-owned reason returned at the public apply boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RejectionReason {
    BeforeHistoryHorizon,
    BeforeSafeTime,
    BeforeSourceTime,
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
    pub(crate) inputs: Vec<SharedEvent<I>>,
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
    pub(crate) response: Sender<Vec<SharedEvent<I>>>,
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
    Prune(Advance<I::Time>),
    Fence { round: u64, observed: Sender<u64> },
    Internal { source: I::Time, batch: RouterBatch<I> },
    Report { round: u64, worker: usize, minimum: Option<I::Time> },
    Pruned { worker: usize, horizon: I::Time },
    SubscribePrunedHorizon(Sender<I::Time>),
    Shutdown,
}

/// One snapshot-specific route emitted by a router.
pub struct Route<I>
where
    I: Input,
{
    pub(crate) snapshot_id: u128,
    pub(crate) input: SharedEvent<I>,
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
    Prune(Advance<I::Time>),
    Fence { round: u64, router: usize },
}

pub(crate) struct History<I>
where
    I: Input,
{
    pub(crate) events: contime_events::EventHistory<SharedEvent<I>>,
}

pub(crate) enum HistoryIter<'a, I>
where
    I: Input,
{
    All(contime_events::EventHistoryIter<'a, SharedEvent<I>>),
    Range(contime_events::EventHistoryRangeIter<'a, SharedEvent<I>>),
}

pub(crate) struct CheckpointStorageConfig {
    pub(crate) checkpoints: crate::checkpoints::CheckpointConfig,
}

pub(crate) struct CheckpointStorage<I, S, W>
where
    I: Input,
    S: contime_checkpoints::Snapshot<Time = I::Time>,
{
    pub(crate) snapshot_id: u128,
    pub(crate) horizon: I::Time,
    pub(crate) interval: u64,
    pub(crate) store: Option<contime_checkpoints::SnapshotStore<S, History<I>>>,
    pub(crate) wrapper: PhantomData<fn() -> W>,
}

/// Deterministic router execution supplied to the runtime.
#[derive(Clone, Debug)]
pub struct RouterProcess<I, S>
where
    I: Input,
{
    pub(crate) placement: contime_router::Placement,
    pub(crate) activity: Receiver<Sender<bool>>,
    pub(crate) controls: Receiver<contime_router::Flush>,
    pub(crate) input: PhantomData<fn() -> (I, S)>,
}

/// Replay worker execution supplied to the runtime.
pub struct WorkerProcess<I, S, W>
where
    I: Input,
    S: contime_checkpoints::Snapshot,
{
    pub(crate) worker: contime_worker::WorkerConfig,
    pub(crate) checkpoints: crate::checkpoints::CheckpointConfig,
    pub(crate) history_retention: I::Time,

    pub(crate) wrapper: W,
    pub(crate) activity: Receiver<Sender<bool>>,
    pub(crate) coordination: Option<contime_worker::Coordination<I::Time>>,
    pub(crate) types: PhantomData<fn() -> (I, S)>,
}

/// Complete apply-and-query process configuration.
#[derive(Clone, Debug)]
pub struct ConTimeConfig<T> {
    pub router_count: usize,
    pub worker_count: usize,
    pub placement: contime_router::Placement,
    /// Minimum wall-clock spacing between safe-pruning measurement rounds.
    /// Zero starts the next required round immediately; safety fences still apply.
    pub pruning_interval: std::time::Duration,
    pub history_retention: T,
    pub worker: contime_worker::WorkerConfig,
    pub checkpoints: crate::checkpoints::CheckpointConfig,
}

/// A running apply-and-query pipeline.
pub struct ConTime<I, S, W>
where
    I: Input,
{
    pub(crate) runtime: contime_runtime::Runtime<RouterMessage<I, S>, contime_router::RouterError, std::convert::Infallible>,
    pub(crate) input: Sender<RouterMessage<I, S>>,
    pub(crate) coordinator: std::thread::JoinHandle<()>,
    pub(crate) errors: Receiver<contime_api::RejectionMessage<RejectionReason>>,
    pub(crate) error_sender: Sender<contime_api::RejectionMessage<RejectionReason>>,

    pub(crate) subscriptions: Vec<Sender<Sender<bool>>>,
    pub(crate) queues: Vec<Arc<dyn Fn() -> bool + Send + Sync>>,
    pub(crate) types: PhantomData<fn() -> (S, W)>,
}
