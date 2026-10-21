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
    /// Returns a stable identity for duplicate detection within each history.
    ///
    /// Retained events are ordered by timestamp and this ID. Reusing an ID for
    /// different payloads in one history can cause the later input to be ignored.
    fn event_id(&self) -> u128;
    /// Emits the snapshot histories affected by this input.
    ///
    /// Routing must be deterministic for the same input. Every emitted ID creates
    /// a route, including duplicates; emitting no IDs drops the input from routing.
    /// Histories share immutable event ownership and deduplicate retained event IDs.
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}

/// An immutable event shared by its routed snapshot histories.
pub struct SharedEvent<I>
where
    I: Input,
{
    pub(crate) inner: Arc<I>,
    pub(crate) observation: Option<crate::frontier::AdmissionObservation<I::Time>>,
}

/// A core-owned reason returned at the public apply boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RejectionReason {
    /// An external input precedes the requested retention boundary, or a target
    /// history can no longer insert an input at its timestamp.
    BeforeHistoryHorizon,
    /// Internally generated work precedes the coordinator's committed safe boundary.
    BeforeSafeTime,
    /// Internally generated work precedes the timestamp of its causal source.
    BeforeSourceTime,
}

/// Request completion forwarded unchanged from API admission to the worker.
#[derive(Clone)]
pub struct CompletionHandle {
    pub(crate) sender: Sender<contime_api::RejectionMessage<RejectionReason>>,
}

/// A processing or pruning timestamp paired with a closure-based completion.
///
/// Protocol adapters construct and consume this opaque payload through their
/// advancement traits. Dropping every completion sender clone ends the request.
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

/// An opaque query for boxed snapshots at one inclusive timestamp.
///
/// API and router query traits construct and partition this request; worker
/// response senders close when all requested histories have been checked.
pub struct SnapshotQuery<T, S> {
    pub(crate) time: T,
    pub(crate) snapshot_ids: Vec<u128>,
    pub(crate) response: Sender<Vec<Box<S>>>,
}

/// An opaque query for retained events in one history over `[from, to)`.
///
/// Results retain shared ownership of the original immutable inputs.
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
    /// A worker has accepted its portion of the requested registration.
    /// Multiple workers may acknowledge disjoint ID subsets separately.
    Registered {
        /// Watched timestamp supplied by the registration.
        time: T,
        /// Snapshot IDs installed by the acknowledging worker.
        snapshot_ids: Vec<u128>,
    },
    /// Relevant replay has affected registered histories at the watched time.
    Replayed {
        /// Watched timestamp, rather than the timestamp of an individual event.
        time: T,
        /// Registered histories affected by the replay update.
        snapshot_ids: Vec<u128>,
    },
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

/// Messages accepted by admission coordination and its downstream routers.
///
/// Coordination consumes internal admissions, worker reports, pruning subscriptions
/// and shutdown. Those variants must not be sent directly to a router process.
pub enum RouterMessage<I, S>
where
    I: Input,
{
    /// Submit an external batch for admission and routing.
    Apply(RouterBatch<I>),
    /// Reconstruct requested snapshots at one timestamp without publishing effects.
    SnapshotQuery(SnapshotQuery<I::Time, S>),
    /// Read retained events from one snapshot history.
    EventQuery(EventQuery<I::Time, I>),
    /// Register interest in replay affecting snapshots at a watched timestamp.
    SnapshotListen(SnapshotListen<I::Time>),
    /// Raise the processing target and request subsequent safe retention progress.
    Advance(Advance<I::Time>),
    /// Forward a proven pruning boundary to the owning workers.
    Prune(Advance<I::Time>),
    /// Resume participants after a measurement and apply its safe pruning bound.
    Resolve {
        /// Measurement identity being resolved.
        round: u64,
        /// Safe boundary and completion handle for distributed resolution.
        prune: Advance<I::Time>,
    },
    /// Mark ordered admission output before router flush controls are issued.
    Fence {
        /// Identity of the measurement requiring this marker.
        round: u64,
        /// Receives the round ID when a router observes the marker.
        observed: Sender<u64>,
    },
    /// Admit feedback with the additional causal lower bound of its source.
    Internal {
        /// Earliest timestamp permitted by the generating application work.
        source: I::Time,
        /// Generated inputs awaiting admission.
        batch: RouterBatch<I>,
    },
    /// Report one worker's fenced earliest remaining work to the coordinator.
    Report {
        /// Measurement identity; stale rounds are ignored.
        round: u64,
        /// Zero-based worker index in the configured participant set.
        worker: usize,
        /// Earliest possible work, or `None` when the worker has none.
        minimum: Option<I::Time>,
    },
    /// Confirm a worker's completed retention boundary.
    Pruned {
        /// Zero-based worker index in the configured participant set.
        worker: usize,
        /// Boundary through which this worker has completed pruning.
        horizon: I::Time,
    },
    /// Receive the current completed pruning horizon and later increases.
    /// The coordinator reports the minimum completed horizon across all workers.
    SubscribePrunedHorizon(Sender<I::Time>),
    /// Stop the admission coordinator on receipt.
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

/// Routed requests consumed by one worker owning a set of snapshot histories.
pub enum WorkerMessage<I, S>
where
    I: Input,
{
    /// Insert the routes belonging to this worker and schedule affected histories.
    Apply(WorkerBatch<I>),
    /// Reconstruct snapshots without publishing live replay effects.
    SnapshotQuery(SnapshotQuery<I::Time, S>),
    /// Read the retained events in one owned history.
    EventQuery(EventQuery<I::Time, I>),
    /// Install this worker's portion of a replay listener registration.
    SnapshotListen(SnapshotListen<I::Time>),
    /// Raise this worker's processing target.
    Advance(Advance<I::Time>),
    /// Move retained history to a coordinator-proven safe boundary.
    Prune(Advance<I::Time>),
    /// Resume processing after a measurement and apply its pruning boundary.
    Resolve {
        /// Identity of the measurement being resolved.
        round: u64,
        /// Safe retention boundary and closure-based completion.
        prune: Advance<I::Time>,
    },
    /// Mark this router's ordered input boundary for a measurement.
    Fence {
        /// Identity of the measurement requiring the fence.
        round: u64,
        /// Zero-based router index in the worker's participant set.
        router: usize,
    },
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
    pub(crate) checkpoints: crate::checkpoints::CheckpointConfig,

    pub(crate) wrapper: W,
    pub(crate) activity: Receiver<Sender<bool>>,
    pub(crate) coordination: Option<contime_worker::Coordination<I::Time>>,
    pub(crate) types: PhantomData<fn() -> (I, S)>,
}
