use crate::RejectionReason;
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

use crossbeam_channel::{Receiver, Sender};

/// An immutable event shared by its routed snapshot histories.
pub struct SharedEvent<I>
where
    I: crate::ThreadInput,
{
    pub(crate) inner: contime_core::SharedEvent<I>,
    pub(crate) observation: Option<crate::frontier::AdmissionObservation<I::Time>>,
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
    I: crate::ThreadInput,
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
    I: crate::ThreadInput,
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

/// Runtime adapter around a consumer-owned notification sender.
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
    I: crate::ThreadInput,
{
    /// Submit an external batch for admission and routing.
    Apply(RouterBatch<I>),
    /// Already-admitted transport segments retaining independent response ownership.
    /// The native coordinator creates bounded packets; public admissions use `Apply`.
    ApplyPacket(Vec<RouterBatch<I>>),
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
    I: crate::ThreadInput,
{
    pub(crate) snapshot_id: u128,
    pub(crate) input: SharedEvent<I>,
}

/// One routed apply batch consumed by a worker.
pub struct WorkerBatch<I>
where
    I: crate::ThreadInput,
{
    pub(crate) routes: Vec<Route<I>>,
    pub(crate) completion: CompletionHandle,
}

/// Routed requests consumed by one worker owning a set of snapshot histories.
pub enum WorkerMessage<I, S>
where
    I: crate::ThreadInput,
{
    /// Insert the routes belonging to this worker and schedule affected histories.
    Apply(WorkerBatch<I>),
    /// Insert independent request segments before granting a replay opportunity.
    /// Each segment releases only its own insertion-completion ownership.
    ApplyPacket(Vec<WorkerBatch<I>>),
    /// Reconstruct snapshots without publishing live replay effects.
    SnapshotQuery(SnapshotQuery<I::Time, S>),
    /// Read the retained events in one owned history.
    EventQuery(EventQuery<I::Time, I>),
    /// Install this worker's portion of a replay listener registration.
    SnapshotListen(SnapshotListen<I::Time>),
    /// Observe worker-owned state without advancing replay or retention.
    Status {
        /// Zero-based worker index in this runtime.
        worker_id: usize,
        /// Dedicated response stream; disconnected receivers are ignored.
        response: Sender<contime_worker::WorkerStatus<I::Time>>,
    },
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

pub(crate) use contime_core::CheckpointStorageConfig;

pub(crate) struct CheckpointStorage<I: crate::ThreadInput, S: contime_checkpoints::Snapshot<Time = I::Time>, W> {
    pub(crate) snapshot_id: u128,
    pub(crate) inner: contime_core::CheckpointStorage<I, S, W>,
}

/// Deterministic router execution supplied to the runtime.
#[derive(Clone, Debug)]
pub struct RouterProcess<I, S>
where
    I: crate::ThreadInput,
{
    pub(crate) placement: contime_router::Placement,
    pub(crate) activity: Receiver<Sender<bool>>,
    pub(crate) controls: Receiver<contime_router::Flush>,
    pub(crate) input: PhantomData<fn() -> (I, S)>,
}

/// Replay worker execution supplied to the runtime.
pub struct WorkerProcess<I, S, W>
where
    I: crate::ThreadInput,
    S: contime_checkpoints::Snapshot,
{
    pub(crate) checkpoints: crate::checkpoints::CheckpointConfig,

    pub(crate) wrapper: W,
    pub(crate) activity: Receiver<Sender<bool>>,
    pub(crate) coordination: Option<contime_worker::Coordination<I::Time>>,
    pub(crate) types: PhantomData<fn() -> (I, S)>,
}
