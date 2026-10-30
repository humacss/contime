/// One event route already assigned to this worker.
#[derive(Debug)]
pub struct RoutedInput<I> {
    /// History identifier already assigned to this worker by placement.
    pub snapshot_id: u128,
    /// Input ownership transferred into that history’s store.
    pub input: I,
}

/// A caller-selected route consumed by a worker.
pub trait RouteInput {
    /// Input value consumed by [`SnapshotStore::insert`].
    type Input;

    /// Transfers the destination history ID and its input without re-routing.
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
    /// Routes inserted in vector order before completion ownership is released.
    pub inputs: Vec<RoutedInput<I>>,
    /// Insertion completion handle; it does not await scheduled replay.
    pub completion: C,
}

/// A caller-selected apply message consumed by a worker.
pub trait ApplyInput {
    /// Owned route carrying an exact snapshot ID and input.
    type Route: RouteInput;
    /// Request handle implementing [`Completion`] for store rejections.
    type Completion;

    /// Transfers the ordered routes and this worker’s insertion completion share.
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
    /// Consumes the handle to report all rejections collected in this batch.
    ///
    /// The Crossbeam sender implementation ignores a disconnected receiver.
    /// A channel receiver must drain all rejection messages until every sender
    /// share drops; closure alone can also result from worker failure.
    fn reject(self, rejections: Vec<R>);
}

impl<R> Completion<R> for crossbeam_channel::Sender<Vec<R>> {
    fn reject(self, rejections: Vec<R>) {
        let _ = crossbeam_channel::Sender::send(&self, rejections);
    }
}

/// The result of inserting one input into the canonical event store.
#[derive(Debug)]
pub struct EventInsert<R> {
    /// Whether canonical history changed and replay must be scheduled or rewound.
    /// Duplicates must return `false` so existing progress is preserved.
    pub changed: bool,
    /// Rejections aggregated into the apply request’s completion response.
    pub rejections: Vec<R>,
}

/// One snapshot whose completed replay may affect every time at or after
/// `affected_from`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayUpdate<T> {
    /// History whose replay completed a processing step.
    pub snapshot_id: u128,
    /// Inclusive earliest time whose state may have changed after this replay.
    pub affected_from: T,
}

pub use crate::contime_time::AdvanceTime;

/// Transfers a processing target or explicitly authorized pruning horizon.
///
/// The operation kind determines whether the time raises processing or retention.
pub trait AdvanceInput {
    /// Ordered time domain shared with the store.
    type Time: AdvanceTime;
    /// Owned handle dropped after accepting a target or completing a prune pass.
    type Completion;

    /// Consumes the request and transfers its time and completion ownership.
    fn into_parts(self) -> (Self::Time, Self::Completion);
}

/// Reports safe-pruning measurements and successfully completed pruning passes.
pub struct Coordination<T> {
    /// Number of distinct router IDs required to fence each measurement round.
    /// Router IDs outside `0..router_count` are ignored.
    pub router_count: usize,
    /// Receives the round and earliest possible replay publication among pending
    /// histories, or `None` when none can publish. Called once after all router
    /// fences arrive; replay then pauses until a matching resolution.
    pub report: Box<dyn FnMut(u64, Option<T>) + Send>,
    /// Called only after every history and retention hook in a pass finishes.
    pub pruned: Box<dyn FnMut(T) + Send>,
}

/// Consumes a point-in-time query over this worker’s histories.
pub trait SnapshotQueryInput {
    /// Query time in the store’s ordered domain.
    type Time;
    /// Response handle implementing [`SnapshotQueryResponse`].
    type Response;

    /// Transfers query time, ordered history IDs, and response ownership.
    /// Missing or unreconstructible histories are omitted from the response.
    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response);
}

/// Consumes an event-range query for one worker-owned history.
pub trait EventQueryInput {
    /// Range-bound type shared with the store.
    type Time;
    /// Response handle implementing [`EventQueryResponse`].
    type Response;

    /// Transfers history ID, inclusive lower bound, exclusive upper bound, and
    /// response ownership. Bounds are passed unchanged to the store.
    fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response);
}

/// A worker-owned timestamped snapshot replay listener.
pub trait SnapshotListener<T>: Clone {
    /// Reports installation with sorted, unique collection IDs. Registrations
    /// with no IDs do not invoke this method.
    /// Returns `false` when the receiver no longer exists.
    fn registered(&self, time: T, snapshot_ids: Vec<u128>) -> bool;

    /// Reports members whose completed replay can affect the watched `time`:
    /// the replay’s affected lower bound is less than or equal to it. This is
    /// an invalidation observation, not a promise that processing reached `time`.
    /// Returns `false` when the receiver no longer exists.
    fn replayed(&self, time: T, snapshot_ids: Vec<u128>) -> bool;
}

/// Caller-selected snapshot-listener registration consumed by a worker.
pub trait SnapshotListenInput {
    /// Watched time compared inclusively with replay’s affected lower bound.
    type Time: Clone + Ord;
    /// Listener retained until a notification indicates its receiver is gone.
    type Listener: SnapshotListener<Self::Time>;

    /// Transfers one watched time, collection IDs, and listener ownership.
    /// The worker sorts and deduplicates IDs; empty collections are not installed.
    fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener);
}

/// Consumes a nonempty batch of independently reconstructed snapshot values.
///
/// Results preserve requested ID order after missing histories are omitted.
/// No call is made for an empty result; its handle is simply dropped.
pub trait SnapshotQueryResponse<S> {
    /// Transfers results and consumes response ownership. The Crossbeam sender
    /// implementation ignores receiver disconnection.
    fn send(self, snapshots: Vec<Box<S>>);
}

impl<S> SnapshotQueryResponse<S> for crossbeam_channel::Sender<Vec<Box<S>>> {
    fn send(self, snapshots: Vec<Box<S>>) {
        let _ = crossbeam_channel::Sender::send(&self, snapshots);
    }
}

/// Consumes a nonempty retained-event result in the store’s canonical order.
///
/// Missing histories and empty ranges drop the response handle without a call.
pub trait EventQueryResponse<I> {
    /// Transfers cloned events and consumes response ownership. The Crossbeam
    /// sender implementation ignores receiver disconnection.
    fn send(self, events: Vec<I>);
}

impl<I> EventQueryResponse<I> for crossbeam_channel::Sender<Vec<I>> {
    fn send(self, events: Vec<I>) {
        let _ = crossbeam_channel::Sender::send(&self, events);
    }
}

/// Cumulative event-to-history application counts for one worker.
///
/// These count canonical insertions and live replay traversals, including repeated
/// replay after late input. Queries and retention forwarding are excluded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerCounters {
    /// Accepted changes to individual histories, excluding duplicates/rejections.
    pub inserted_applications: u64,
    /// Live replay traversals, or `None` after a store cannot report its count.
    pub replay_applications: Option<u64>,
    /// At least one available cumulative count overflowed and was clamped.
    pub saturated: bool,
}

impl Default for WorkerCounters {
    fn default() -> Self {
        Self { inserted_applications: 0, replay_applications: Some(0), saturated: false }
    }
}

#[cfg(feature = "monitor")]
impl WorkerCounters {
    pub(crate) fn add_inserted(&mut self, count: u64) {
        self.inserted_applications = Self::add(self.inserted_applications, count, &mut self.saturated);
    }

    pub(crate) fn add_replay(&mut self, count: Option<u64>) {
        self.replay_applications = match (self.replay_applications, count) {
            (Some(total), Some(count)) => Some(Self::add(total, count, &mut self.saturated)),
            _ => None,
        };
    }

    fn add(total: u64, count: u64, saturated: &mut bool) -> u64 {
        match total.checked_add(count) {
            Some(total) => total,
            None => {
                *saturated = true;
                u64::MAX
            }
        }
    }
}

/// Read-only gauges captured between operations on one worker.
///
/// Gauges describe orchestration state; optional counters describe cumulative
/// event-to-history applications. Reports from separate workers need not describe the same instant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerStatus<T> {
    /// Identifier supplied by the status request's topology owner.
    pub worker_id: usize,
    /// Entries owning an initialized history store.
    pub history_stores: usize,
    /// Entries with notification metadata but no initialized history store.
    pub metadata_only: usize,
    /// Histories scheduled for replay, including future work and blocked work.
    pub pending_histories: usize,
    /// Scheduled histories at or before the processing target.
    /// Pruning or coordination may still prevent them from running.
    pub due_histories: usize,
    /// Accepted processing target; this does not mean replay has reached it.
    pub processing_target: T,
    /// Accepted admission horizon; pruning through it may still be queued.
    pub admission_horizon: T,
    /// Queued pruning passes, including a partly serviced oldest pass.
    pub pending_prune_passes: usize,
    /// Reported coordination round whose resolution is needed before replay.
    pub awaiting_resolution: Option<u64>,
    /// Optional cumulative counts; `None` when the `monitor` feature is disabled.
    pub counters: Option<WorkerCounters>,
}

/// Operations serviced serially by one worker’s message loop.
pub enum WorkInputKind<A, SQ, EQ, SL, AD: AdvanceInput> {
    /// Captures orchestration gauges without querying stores or driving work.
    /// A full or disconnected response channel drops the report without blocking.
    Status {
        /// Topology owner’s worker index, copied into the report.
        worker_id: usize,
        /// Receives one report with a nonblocking send.
        response: crossbeam_channel::Sender<WorkerStatus<AD::Time>>,
    },
    /// Inserts a complete batch and releases its completion before replay.
    Apply(A),
    /// Inserts adjacent segments, releasing each response independently before replay.
    ApplyPacket(Vec<A>),
    /// Reconstructs snapshots without publishing effects or advancing replay.
    SnapshotQuery(SQ),
    /// Reads retained events without altering the schedule.
    EventQuery(EQ),
    /// Installs a collection watching state changes at one time.
    SnapshotListen(SL),
    /// Raises the processing target monotonically; completion acknowledges acceptance.
    Advance(AD),
    /// Records one router’s fence; duplicates and older reported rounds are ignored.
    Fence {
        /// Measurement round, monotonically increasing across completed rounds.
        round: u64,
        /// Router ID in the coordination configuration’s index range.
        router: usize,
    },
    /// Releases a reported round, forwarding to its authorized horizon before replay.
    Resolve {
        /// Must match the round currently awaiting resolution; others are dropped.
        round: u64,
        /// Authorized horizon and completion held until queued pruning finishes.
        prune: AD,
    },
    /// Authorizes a prune pass; it must not discard pending replay.
    Prune(AD),
}

/// Converts caller-owned messages into the worker’s static protocol.
pub trait WorkInput {
    /// Batch payload implementing [`ApplyInput`].
    type Apply;
    /// Point query payload implementing [`SnapshotQueryInput`].
    type SnapshotQuery;
    /// Event-range query payload implementing [`EventQueryInput`].
    type EventQuery;
    /// Listener collection payload implementing [`SnapshotListenInput`].
    type SnapshotListen;
    /// Target or pruning payload implementing [`AdvanceInput`].
    type Advance: AdvanceInput;

    /// Consumes the message and transfers its operation payload.
    // Keep the five caller-selected message types explicit at this boundary.
    #[allow(clippy::type_complexity)]
    fn into_kind(self) -> WorkInputKind<Self::Apply, Self::SnapshotQuery, Self::EventQuery, Self::SnapshotListen, Self::Advance>;
}

/// One generational index into worker-local notification collection storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NotificationId {
    pub(crate) index: usize,
    pub(crate) generation: u64,
}

/// Storage operations needed by the timestamp-driven worker.
/// The orchestrator adapts its snapshot store to this contract.
pub trait SnapshotStore<I>: Sized {
    /// Shared immutable configuration used to construct each history lazily.
    type Config;
    /// Worker-owned mutable application and forwarding context.
    type Context;
    /// Ordered timestamp domain; its default initializes target and horizon.
    type Time: AdvanceTime;
    /// Independently owned state returned by historical reconstruction.
    type Snapshot;
    /// Rejection payload collected into request-scoped completion responses.
    type Rejection;

    /// Creates one history with the current admission horizon, including when a
    /// pruning pass has been queued but has not finished forwarding older stores.
    fn create(snapshot_id: u128, config: &Self::Config, horizon: &Self::Time) -> Self;
    /// Reject inputs before admission_horizon, including while forwarding is queued.
    /// A duplicate must not invalidate progress and must return changed = false.
    fn insert(&mut self, input: I, admission_horizon: &Self::Time) -> EventInsert<Self::Rejection>;
    /// Timestamp of an input, used by the worker to schedule changed histories.
    fn event_time(input: &I) -> Self::Time;
    /// Earliest remaining replay application, including any unchanged prefix
    /// reconstructed from the selected checkpoint after invalidation.
    /// `None` means no accepted event remains to replay, including future events.
    ///
    /// The worker uses this timestamp for scheduling and fenced pruning reports.
    /// After `process_until(T)` finishes, a returned timestamp must be strictly
    /// greater than `T`. Insertion may invalidate history and move it backward;
    /// queries must not change it.
    fn earliest_replay_time(&self) -> Option<Self::Time>;
    /// Process all remaining timestamp batches through time, inclusively.
    /// On return, `earliest_replay_time()` must be `None` or later than `time`.
    fn process_until(&mut self, time: &Self::Time, context: &mut Self::Context);
    /// Processes the same inclusive endpoint and reports raw live replay traversals.
    ///
    /// Include repeated canonical traversals after late insertion; exclude query
    /// reconstruction and retention forwarding. `None` means counting is unavailable.
    /// The default preserves custom stores by calling [`Self::process_until`].
    #[cfg(feature = "monitor")]
    fn process_until_counted(&mut self, time: &Self::Time, context: &mut Self::Context) -> Option<u64> {
        self.process_until(time, context);
        None
    }
    /// Reconstruct fresh state through `time`, including events exactly at it,
    /// without publishing effects or changing replay progress or scheduled work.
    /// Return `None` when retained state cannot reconstruct the requested time.
    fn query(&mut self, time: Self::Time, context: &mut Self::Context) -> Option<Box<Self::Snapshot>>;
    /// Clones retained events in canonical order in `[from, to)`.
    /// The lower bound is inclusive and the upper bound exclusive. Return no
    /// events for an empty or reversed interval; do not mutate replay progress.
    fn query_events(&self, from: &Self::Time, to: &Self::Time) -> Vec<I>
    where
        I: Clone;
    /// Establish the retained checkpoint needed at `horizon`, including
    /// forwarding/retention hooks, before [`Self::prune`] removes old history.
    /// Must preserve reconstruction of the retained suffix without republishing
    /// old application effects. A panic prevents successful pass reporting.
    fn forward(&mut self, horizon: &Self::Time, context: &mut Self::Context);
    /// Remove history made unnecessary by the last successful forward.
    fn prune(&mut self);
}

pub(crate) struct StoreSlot<S> {
    pub(crate) store: Option<S>,
    pub(crate) notification_ids: Vec<NotificationId>,
}
impl<S> StoreSlot<S> {
    pub(crate) fn metadata_only() -> Self {
        Self { store: None, notification_ids: Vec::new() }
    }
}

#[cfg(all(test, feature = "monitor"))]
mod counter_tests {
    use super::WorkerCounters;

    #[test]
    fn cumulative_counts_saturate_and_keep_the_overflow_signal() {
        let mut counters =
            WorkerCounters { inserted_applications: u64::MAX - 1, replay_applications: Some(u64::MAX - 1), saturated: false };
        counters.add_inserted(1);
        counters.add_replay(Some(1));
        assert!(!counters.saturated, "reaching the maximum exactly is representable");
        counters.add_inserted(1);
        counters.add_replay(Some(2));
        assert_eq!(counters.inserted_applications, u64::MAX);
        assert_eq!(counters.replay_applications, Some(u64::MAX));
        assert!(counters.saturated);
        counters.add_replay(None);
        counters.add_replay(Some(1));
        counters.add_inserted(0);
        assert_eq!(counters.replay_applications, None);
        assert!(counters.saturated);
    }

    #[test]
    fn replay_overflow_independently_marks_saturation() {
        let mut counters = WorkerCounters { replay_applications: Some(u64::MAX), ..Default::default() };
        counters.add_replay(Some(1));
        assert_eq!(counters.inserted_applications, 0);
        assert_eq!(counters.replay_applications, Some(u64::MAX));
        assert!(counters.saturated);
    }

    #[test]
    fn unknown_replay_is_sticky_without_claiming_overflow() {
        let mut counters = WorkerCounters::default();
        counters.add_replay(Some(4));
        counters.add_replay(None);
        counters.add_replay(Some(7));
        assert_eq!(counters.replay_applications, None);
        assert!(!counters.saturated);
    }
}
