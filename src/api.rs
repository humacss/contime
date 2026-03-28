use flume::Receiver;

use crate::handle::{AdvanceHandle, ApplyHandle, HandleError, QueryHandle, QueryResult};
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
pub struct Contime<SL: SnapshotLanes<Event = EL>, EL: EventLanes<SL>> {
    router: Router<SL, EL>,
}

impl<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL> + 'static> Contime<SL, EL> {
    /// Creates a `contime` instance with `worker_count` workers and a shared memory budget.
    ///
    /// Most users generate `SL` and `EL` with [`crate::contime!`].
    pub fn new(worker_count: usize, memory_budget_bytes: u64) -> Self {
        let router = Router::<SL, EL>::new(worker_count, memory_budget_bytes);

        Self { router }
    }

    /// Creates a `contime` instance that retains a bounded amount of history behind the
    /// internally advanced current time.
    ///
    /// Call [`Contime::advance`] to move the current time forward. History older than
    /// `current_time - lower_time_horizon_delta` becomes eligible for pruning.
    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: i64) -> Self {
        let router = Router::<SL, EL>::with_history_horizon(worker_count, memory_budget_bytes, lower_time_horizon_delta);

        Self { router }
    }

    // Sync methods (blocking)

    /// Advances the internal current time by `time`.
    ///
    /// When history pruning is enabled with [`Contime::with_history_horizon`], advancing can
    /// free old checkpoints and events.
    pub fn advance(&self, time: i64) -> Result<(), ContimeError> {
        Ok(self.router.advance(time)?)
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
}
