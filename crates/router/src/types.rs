/// Default owned input batch for apply-only or unified routing.
#[derive(Debug)]
pub struct InputBatch<I, C> {
    /// Inputs in their original visitation order; each may emit zero or more routes.
    pub inputs: Vec<I>,
    /// Request ownership cloned once per additional affected worker.
    /// An empty route set drops it without emitting a worker message.
    pub completion: C,
}

/// A caller-selected batch consumed by the router.
pub trait RouteInputBatch {
    /// Input value whose snapshot routes are visited before it is moved or cloned.
    type Input;
    /// Cloneable request ownership carried to every affected worker.
    type Completion;

    /// Consumes the batch and transfers its inputs and completion ownership.
    fn into_parts(self) -> (Vec<Self::Input>, Self::Completion);
}

impl<I, C> RouteInputBatch for InputBatch<I, C> {
    type Input = I;
    type Completion = C;

    fn into_parts(self) -> (Vec<Self::Input>, Self::Completion) {
        (self.inputs, self.completion)
    }
}

/// Default record pairing one snapshot history with its owned input.
#[derive(Debug, PartialEq, Eq)]
pub struct RoutedInput<I> {
    /// Exact history identifier; the worker need not repeat placement.
    pub snapshot_id: u128,
    /// Input moved to this route; additional routes receive clones.
    pub input: I,
}

/// Constructs one caller-selected route emitted by the router.
pub trait RouteOutput<I>: Sized {
    /// Takes ownership of an input for the specified history.
    fn create(snapshot_id: u128, input: I) -> Self;
}

impl<I> RouteOutput<I> for RoutedInput<I> {
    fn create(snapshot_id: u128, input: I) -> Self {
        Self { snapshot_id, input }
    }
}

/// Default apply message containing all routes for one worker from one input batch.
#[derive(Debug)]
pub struct WorkerBatch<I, C> {
    /// Routes assigned to this worker, preserving input and emitted-route order.
    pub inputs: Vec<RoutedInput<I>>,
    /// This worker’s share of request completion ownership.
    pub completion: C,
}

/// Constructs one caller-selected worker message emitted by the router.
pub trait WorkerOutput<I, C>: Sized {
    /// Owned record used for each snapshot destination in the final worker batch.
    type Route: RouteOutput<I>;

    /// Consumes the final nonempty worker partition and its completion share.
    fn create(inputs: Vec<Self::Route>, completion: C) -> Self;

    /// Packs apply messages constructed by `create` for one worker into transports.
    /// The default preserves individual messages. Adapters can group segments while
    /// preserving each segment's rejection destination and completion ownership.
    fn coalesce(messages: Vec<Self>) -> Vec<Self> {
        messages
    }
}

impl<I, C> WorkerOutput<I, C> for WorkerBatch<I, C> {
    type Route = RoutedInput<I>;

    fn create(inputs: Vec<Self::Route>, completion: C) -> Self {
        Self { inputs, completion }
    }
}

/// Dispatch failure; previously sent worker messages are not rolled back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouterError {
    /// The supplied worker collection is empty; no dispatch was attempted.
    NoWorkers,
    /// A worker receiver disconnected before accepting its message.
    WorkerUnavailable {
        /// Zero-based index in the supplied worker sender collection.
        worker_index: usize,
    },
}

/// Enumerates the histories affected by one input.
///
/// Emission order is retained within each worker partition. Duplicate IDs are
/// not deduplicated, and no emitted IDs means the input is dropped.
pub trait RoutableInput {
    /// Calls `emit` once per desired route without transferring input ownership.
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}

/// Caller-selected historical snapshot query consumed by the router.
pub trait SnapshotQueryInput {
    /// Timestamp reconstructed by the destination worker.
    type Time;
    /// Response ownership cloned across affected workers; aggregation is caller-owned.
    type Response: Clone;

    /// Transfers query time, requested IDs, and response ownership.
    /// IDs are partitioned without sorting or deduplication.
    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response);
}

/// Caller-selected snapshot-listener registration consumed by the router.
pub trait SnapshotListenInput {
    /// Watched time, cloned once per additional affected worker.
    type Time: Clone;
    /// Listener ownership shared by affected worker partitions through `Clone`.
    type Listener: Clone;

    /// Transfers one timestamped listener collection and its requested IDs.
    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener);
}

/// Caller-selected event-history query consumed by the router.
pub trait EventQueryInput {
    /// Event-range bound type, forwarded without comparison or validation.
    type Time;
    /// Owned response handle moved to the single destination worker.
    type Response;

    /// Transfers history ID, lower bound, upper bound, and response ownership.
    /// The router preserves bounds unchanged; storage defines their interpretation.
    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response);
}

/// Constructs a caller-selected worker snapshot-query message.
pub trait SnapshotQueryWorkerOutput<T, R>: Sized {
    /// Consumes one nonempty worker partition and its share of the query response.
    fn snapshot_query(time: T, snapshot_ids: Vec<u128>, response: R) -> Self;
}

/// Constructs a caller-selected worker listener-registration message.
pub trait SnapshotListenWorkerOutput<T, L>: Sized {
    /// Consumes one nonempty worker partition of a timestamped listener collection.
    fn listen(time: T, snapshot_ids: Vec<u128>, listener: L) -> Self;
}

/// Constructs a caller-selected worker event-query message.
pub trait EventQueryWorkerOutput<T, R>: Sized {
    /// Consumes the exact history ID, unchanged range bounds, and response handle.
    fn event_query(snapshot_id: u128, from: T, to: T, response: R) -> Self;
}

/// Transfers a processing target or pruning horizon with broadcast completion ownership.
pub trait AdvanceInput {
    /// Time value copied to every worker in a broadcast.
    type Time: Clone;
    /// Request ownership copied to every worker; the router does not await it.
    type Completion: Clone;

    /// Consumes the time request and transfers its completion handle.
    fn into_parts(self) -> (Self::Time, Self::Completion);
}

/// Constructs a worker message that raises its processing target.
pub trait AdvanceWorkerOutput<T, C>: Sized {
    /// Transfers the target and this worker’s completion share without waiting for replay.
    fn advance(time: T, completion: C) -> Self;
}

/// Requests an ordered fence after this router's current dispatch.
pub struct Flush {
    /// Coordinator round to attach to every worker fence.
    pub round: u64,
    /// Stable zero-based router identity within the topology.
    pub router: usize,
}

/// Constructs worker coordination messages without enforcing coordinator policy.
pub trait CoordinationOutput<T, C>: Sized {
    /// Marks this router’s completed dispatch for the named round.
    fn fence(round: u64, router: usize) -> Self;
    /// Transfers an authorized pruning horizon and this worker’s completion share.
    fn prune(time: T, completion: C) -> Self;
    /// Carries the round decision even when the authorized horizon is unchanged.
    fn resolve(round: u64, time: T, completion: C) -> Self;
}

/// One of the operations accepted by a unified router queue.
pub enum RouteInputKind<A, SQ, EQ, SL, AD> {
    /// Partitions an input batch by snapshot ownership.
    Apply(A),
    /// Partitions adjacent admitted segments while retaining their completion handles.
    ApplyPacket(Vec<A>),
    /// Partitions a point-in-time snapshot query by ownership.
    SnapshotQuery(SQ),
    /// Routes an event-history range query to one worker.
    EventQuery(EQ),
    /// Partitions one timestamped listener collection by ownership.
    SnapshotListen(SL),
    /// Broadcasts a processing target; it does not authorize pruning.
    Advance(AD),
    /// Broadcasts explicit pruning authorization.
    Prune(AD),
    /// Broadcasts a round decision, including an unchanged authorized horizon.
    Resolve {
        /// Reported coordinator round being released.
        round: u64,
        /// Authorized horizon and completion ownership.
        prune: AD,
    },
    /// Reports dequeue of an in-band round marker; it does not flush other routers.
    Fence {
        /// Round value sent to the observer.
        round: u64,
        /// Receives the round; a disconnected observer is ignored.
        observed: crossbeam_channel::Sender<u64>,
    },
}

/// Converts a caller-selected router message into its static operation kind.
pub trait RouteInput {
    /// Batch payload implementing [`RouteInputBatch`].
    type Apply;
    /// Point query payload implementing [`SnapshotQueryInput`].
    type SnapshotQuery;
    /// Range query payload implementing [`EventQueryInput`].
    type EventQuery;
    /// Registration payload implementing [`SnapshotListenInput`].
    type SnapshotListen;
    /// Target or horizon payload implementing [`AdvanceInput`].
    type Advance;

    /// Consumes the caller’s message and transfers its operation payload.
    // Keep each operation payload explicit at the protocol boundary.
    #[allow(clippy::type_complexity)]
    fn into_kind(self) -> RouteInputKind<Self::Apply, Self::SnapshotQuery, Self::EventQuery, Self::SnapshotListen, Self::Advance>;
}
