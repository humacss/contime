use flume::Receiver;
use std::marker::PhantomData;

use crate::handle::{AdvanceHandle, ApplyHandle, HandleError, QueryHandle, QueryResult, TimeAdvanceSubscription};
use crate::history::Reconciliation;
use crate::{AfterApplyEvents, ApplyContextEvents, ApplyEvents, Event, EventLanes, Router, RouterError, Snapshot, SnapshotLanes};

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
pub struct Contime<SL: SnapshotLanes<Event = EL> + ApplyEvents, EL: EventLanes<SL, C>, C = ()>
where
    SL: AfterApplyEvents<C>,
{
    router: Router<SL, EL, C>,
    apply_context: C,
    _context: PhantomData<C>,
}

impl<SL, EL> Contime<SL, EL, ()>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    SL: AfterApplyEvents<()> + 'static,
    EL: EventLanes<SL> + 'static,
{
    /// Creates a `contime` instance with `worker_count` workers and a shared memory budget.
    ///
    /// Most users generate `SL` and `EL` with [`crate::contime!`].
    pub fn new(worker_count: usize, memory_budget_bytes: u64) -> Self {
        let router = Router::<SL, EL>::new(worker_count, memory_budget_bytes);

        Self { router, apply_context: (), _context: PhantomData }
    }

    /// Creates a `contime` instance that retains a bounded amount of history behind the
    /// internally advanced current time.
    ///
    /// Call [`Contime::advance`] to move the current time forward. History older than
    /// `current_time - lower_time_horizon_delta` becomes eligible for pruning.
    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: i64) -> Self {
        let router = Router::<SL, EL>::with_history_horizon(worker_count, memory_budget_bytes, lower_time_horizon_delta);

        Self { router, apply_context: (), _context: PhantomData }
    }
}

impl<SL, EL, C> Contime<SL, EL, C>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    SL: AfterApplyEvents<C> + 'static,
    EL: EventLanes<SL, C> + 'static,
    C: ApplyContextEvents<EL> + Clone + Send + 'static,
{
    /// Creates a `contime` instance with an explicit per-worker apply context.
    pub fn new_with_apply_context(worker_count: usize, memory_budget_bytes: u64, apply_context: C) -> Self {
        let router = Router::<SL, EL, C>::new_with_apply_context(worker_count, memory_budget_bytes, apply_context.clone());

        Self { router, apply_context, _context: PhantomData }
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
            apply_context.clone(),
        );

        Self { router, apply_context, _context: PhantomData }
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

    /// Returns a clone of the apply context attached to this runtime.
    ///
    /// Extension layers can use this to inspect their own context-owned queues
    /// after public `contime` operations complete.
    pub fn apply_context(&self) -> C {
        self.apply_context.clone()
    }

    /// Applies an event synchronously and waits for all affected workers to finish.
    pub fn apply_event<E: Event>(&self, event: E) -> Result<(), ContimeError>
    where
        EL: From<E>,
    {
        self.router.apply_event(event.into())?;
        self.apply_pending_context_events()
    }

    /// Applies an authoritative snapshot synchronously and replays any later events on top of it.
    pub fn apply_snapshot<S: Snapshot>(&self, snapshot: S) -> Result<(), ContimeError>
    where
        SL: From<S>,
    {
        self.router.apply_snapshot(snapshot.into())?;
        self.apply_pending_context_events()
    }

    /// Returns the snapshot state at `time` together with a reconciliation receiver.
    ///
    /// Events at exactly `time` are included.
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
    /// Events at exactly `time` are included. This batch API does not allocate
    /// reconciliation receivers and is intended for hot internal read paths.
    ///
    /// Results are returned in the same order as `snapshot_ids`. Missing histories yield `None`.
    pub fn many_at(&self, time: i64, snapshot_ids: &[u128]) -> Result<Vec<Option<SL>>, ContimeError> {
        Ok(self.router.many_at(time, snapshot_ids)?)
    }

    /// Returns all known snapshot lanes at `time`, grouped by owning worker.
    ///
    /// Events at exactly `time` are included. This read-only query does not allocate
    /// reconciliation receivers. Workers are returned in worker-index order,
    /// and each worker's lanes are sorted by snapshot id.
    pub fn snapshot_lanes_by_worker(&self, time: i64) -> Result<Vec<Vec<SL>>, ContimeError> {
        Ok(self.router.snapshot_lanes_by_worker(time)?)
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

    /// Broadcasts an advance request to every worker and returns a completion handle.
    pub fn send_advance(&self, time: i64) -> Result<AdvanceHandle, ContimeError> {
        Ok(self.router.send_advance(time)?)
    }

    /// Sends an absolute-time advance request and returns a completion handle.
    pub fn send_advance_to(&self, time: i64) -> Result<AdvanceHandle, ContimeError> {
        Ok(self.router.send_advance_to(time)?)
    }

    /// Subscribes to successful global time advancement.
    pub fn subscribe_time_advances(&self) -> Result<TimeAdvanceSubscription, ContimeError> {
        Ok(self.router.subscribe_time_advances()?)
    }

    fn apply_pending_context_events(&self) -> Result<(), ContimeError> {
        loop {
            let events = self.apply_context.drain_after_apply_events();
            let replacements = self.apply_context.drain_after_apply_replacements();
            if events.is_empty() && replacements.is_empty() {
                return Ok(());
            }
            for event in events {
                self.router.apply_event(event)?;
            }
            for replacement in replacements {
                self.router.replace_context_events(replacement.source_key, replacement.from_time, replacement.events)?;
            }
        }
    }
}
