use flume::Receiver;
use std::marker::PhantomData;

use crate::handle::{
    AdvanceHandle, ApplyHandle, EventQueryHandle, HandleError, QueryEventsResult, QueryHandle, QueryResult, TimeAdvanceSubscription,
    WakeSubscription,
};
use crate::history::Reconciliation;
use crate::{Event, EventLanes, Router, RouterError, Snapshot, SnapshotLanes};

/// Errors returned by [`Contime`] operations.
#[derive(Debug)]
pub enum ContimeError {
    /// Applying the input would exceed the configured memory budget.
    MemoryFull,
    /// The requested snapshot id has no known history.
    NotFound,
    /// A worker stopped before it could complete the request.
    WorkerDropped,
    /// Internal routing error.
    RouterError(RouterError),
}

impl From<RouterError> for ContimeError {
    fn from(err: RouterError) -> Self {
        match err {
            RouterError::MemoryFull => ContimeError::MemoryFull,
            other => ContimeError::RouterError(other),
        }
    }
}

impl From<HandleError> for ContimeError {
    fn from(_: HandleError) -> Self {
        ContimeError::WorkerDropped
    }
}

/// Main entry point for building and querying continuous-time state.
///
/// `SL` and `EL` are usually generated with [`crate::contime!`].
pub struct Contime<SL: SnapshotLanes<Event = EL>, EL: EventLanes<SL, C>, C = ()> {
    router: Router<SL, EL, C>,
    _context: PhantomData<C>,
}

impl<SL, EL> Contime<SL, EL, ()>
where
    SL: SnapshotLanes<Event = EL> + 'static,
    EL: EventLanes<SL> + 'static,
{
    /// Creates a `contime` instance with `worker_count` workers and a shared memory budget.
    ///
    /// Most users generate `SL` and `EL` with [`crate::contime!`].
    pub fn new(worker_count: usize, memory_budget_bytes: u64) -> Self {
        let router = Router::<SL, EL>::new(worker_count, memory_budget_bytes);

        Self { router, _context: PhantomData }
    }

    /// Creates a `contime` instance that retains a bounded amount of history behind the
    /// internally advanced current time.
    ///
    /// Call [`Contime::advance`] to move the current time forward. History older than
    /// `current_time - lower_time_horizon_delta` becomes eligible for pruning.
    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: i64) -> Self {
        let router = Router::<SL, EL>::with_history_horizon(worker_count, memory_budget_bytes, lower_time_horizon_delta);

        Self { router, _context: PhantomData }
    }
}

impl<SL, EL, C> Contime<SL, EL, C>
where
    SL: SnapshotLanes<Event = EL> + 'static,
    EL: EventLanes<SL, C> + 'static,
    C: Clone + Send + 'static,
{
    /// Creates a `contime` instance with an explicit per-worker apply context.
    pub fn new_with_apply_context(worker_count: usize, memory_budget_bytes: u64, apply_context: C) -> Self {
        let router = Router::<SL, EL, C>::new_with_apply_context(worker_count, memory_budget_bytes, apply_context);

        Self { router, _context: PhantomData }
    }

    /// Creates a history-bounded `contime` instance with an explicit per-worker apply context.
    pub fn with_history_horizon_and_apply_context(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: i64,
        apply_context: C,
    ) -> Self {
        let router = Router::<SL, EL, C>::with_history_horizon_and_apply_context(
            worker_count,
            memory_budget_bytes,
            lower_time_horizon_delta,
            apply_context,
        );

        Self { router, _context: PhantomData }
    }

    // Sync methods (blocking)

    /// Advances the internal current time by `time`.
    ///
    /// When history pruning is enabled with [`Contime::with_history_horizon`], advancing can
    /// free old checkpoints and events.
    pub fn advance(&self, time: i64) -> Result<(), ContimeError> {
        Ok(self.router.advance(time)?)
    }

    /// Advances the internal current time to `time` if it is newer.
    ///
    /// This is a convenience wrapper around the delta-based [`Contime::advance`]
    /// operation for hosts that use absolute logical timestamps. Calling this
    /// with a time older than or equal to the current time is a no-op.
    pub fn advance_to(&self, time: i64) -> Result<(), ContimeError> {
        Ok(self.router.advance_to(time)?)
    }

    /// Returns the latest internal current time observed by this runtime.
    pub fn current_time(&self) -> i64 {
        self.router.current_time()
    }

    /// Applies an event synchronously and waits for all affected workers to finish.
    pub fn apply_event<E: Event>(&self, event: E) -> Result<(), ContimeError>
    where
        EL: From<E>,
    {
        Ok(self.router.apply_event(event.into())?)
    }

    /// Applies an authoritative snapshot synchronously and replays any later events on top of it.
    pub fn apply_snapshot<S: Snapshot>(&self, snapshot: S) -> Result<(), ContimeError>
    where
        SL: From<S>,
    {
        Ok(self.router.apply_snapshot(snapshot.into())?)
    }

    /// Returns the snapshot state at `time` together with a reconciliation receiver.
    ///
    /// Events at exactly `time` are not included. Query the first logical instant after the
    /// event you want included.
    pub fn at<S>(&self, time: i64, snapshot_id: u128) -> Result<(S, Receiver<Reconciliation>), ContimeError>
    where
        S: Snapshot + From<SL>,
    {
        match self.router.at(time, snapshot_id)? {
            QueryResult::Found(snapshot_lane, reconciliation_rx) => Ok((snapshot_lane.into(), reconciliation_rx)),
            QueryResult::NotFound => Err(ContimeError::NotFound),
        }
    }

    /// Returns snapshot lanes for many snapshot ids at the same query time.
    ///
    /// Events at exactly `time` are not included. This batch API does not allocate
    /// reconciliation receivers and is intended for hot internal read paths.
    ///
    /// Results are returned in the same order as `snapshot_ids`. Missing histories yield `None`.
    pub fn many_at(&self, time: i64, snapshot_ids: &[u128]) -> Result<Vec<Option<SL>>, ContimeError> {
        Ok(self.router.many_at(time, snapshot_ids)?)
    }

    /// Returns all known snapshot lanes at `time`, grouped by owning worker.
    ///
    /// Events at exactly `time` are not included. This read-only query does not allocate
    /// reconciliation receivers or emit wakes. Workers are returned in worker-index order,
    /// and each worker's lanes are sorted by snapshot id.
    pub fn snapshot_lanes_by_worker(&self, time: i64) -> Result<Vec<Vec<SL>>, ContimeError> {
        Ok(self.router.snapshot_lanes_by_worker(time)?)
    }

    /// Returns retained events for one snapshot id in a half-open time range.
    ///
    /// Events satisfy `from_time <= event.time() < to_time` and are returned
    /// in deterministic `(time, event_id)` order. This is a retained-history
    /// query only; events pruned by [`Contime::advance`] and the configured
    /// history horizon are not returned.
    pub fn events_between(&self, snapshot_id: u128, from_time: i64, to_time: i64) -> Result<Vec<EL>, ContimeError> {
        match self.router.events_between(snapshot_id, from_time, to_time)? {
            QueryEventsResult::Found(events) => Ok(events),
            QueryEventsResult::NotFound => Err(ContimeError::NotFound),
        }
    }

    // Async methods (handle-based)

    /// Sends an event and returns a handle that can be waited on, polled, or awaited.
    pub fn send_event<E: Event>(&self, event: E) -> Result<ApplyHandle, ContimeError>
    where
        EL: From<E>,
    {
        Ok(self.router.send_event(event.into())?)
    }

    /// Sends an authoritative snapshot and returns a handle for completion.
    pub fn send_snapshot<S: Snapshot>(&self, snapshot: S) -> Result<ApplyHandle, ContimeError>
    where
        SL: From<S>,
    {
        Ok(self.router.send_snapshot(snapshot.into())?)
    }

    /// Starts a query and returns a handle for retrieving the result later.
    pub fn query_at(&self, time: i64, snapshot_id: u128) -> Result<QueryHandle<SL>, ContimeError> {
        Ok(self.router.query_at(time, snapshot_id)?)
    }

    /// Starts a snapshot-scoped event history query and returns a handle.
    ///
    /// The returned events satisfy `from_time <= event.time() < to_time` and
    /// are sorted by `(time, event_id)`.
    pub fn query_events_between(&self, snapshot_id: u128, from_time: i64, to_time: i64) -> Result<EventQueryHandle<EL>, ContimeError> {
        Ok(self.router.query_events_between(snapshot_id, from_time, to_time)?)
    }

    /// Broadcasts an advance request to every worker and returns a completion handle.
    pub fn send_advance(&self, time: i64) -> Result<AdvanceHandle, ContimeError> {
        Ok(self.router.send_advance(time)?)
    }

    /// Sends an absolute-time advance request and returns a completion handle.
    pub fn send_advance_to(&self, time: i64) -> Result<AdvanceHandle, ContimeError> {
        Ok(self.router.send_advance_to(time)?)
    }

    /// Subscribes to the global snapshot wake stream for this contime instance.
    ///
    /// Wakes are emitted for both normal in-order state changes and historical
    /// reconciliation windows. The returned subscription merges wakes across all
    /// internal workers into one unified stream.
    pub fn subscribe_wakes(&self) -> Result<WakeSubscription<SL>, ContimeError> {
        Ok(self.router.subscribe_wakes()?)
    }

    /// Subscribes to successful global time advancement.
    pub fn subscribe_time_advances(&self) -> Result<TimeAdvanceSubscription, ContimeError> {
        Ok(self.router.subscribe_time_advances()?)
    }
}
