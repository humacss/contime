use std::marker::PhantomData;

use crate::{ApplyError, ApplyEvents, ApplyWrapper, Event, EventLanes, Router, RouterError, SnapshotLanes};

/// Errors returned by [`Contime`] operations.
#[derive(Debug)]
pub enum ContimeError {
    /// Applying the input would exceed the configured memory budget.
    MemoryFull,
    /// The requested snapshot id has no known history.
    NotFound,
    /// The apply wrapper rejected the event batch.
    ApplyFailed(ApplyError),
    /// Internal routing error.
    RouterError(RouterError),
}

impl From<RouterError> for ContimeError {
    fn from(err: RouterError) -> Self {
        match err {
            RouterError::MemoryFull => ContimeError::MemoryFull,
            RouterError::ApplyFailed(error) => ContimeError::ApplyFailed(error),
            other => ContimeError::RouterError(other),
        }
    }
}

/// Main entry point for building and querying continuous-time state.
///
/// `SL` and `EL` are usually generated with [`crate::contime!`].
pub struct Contime<SL: SnapshotLanes<Event = EL> + ApplyEvents, EL: EventLanes<SL, C>, C = ()>
where
    C: ApplyWrapper<SL>,
{
    router: Router<SL, EL, C>,
    apply_context: C,
    _context: PhantomData<C>,
}

impl<SL, EL> Contime<SL, EL, ()>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    (): ApplyWrapper<SL>,
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
    C: ApplyWrapper<SL> + 'static,
    EL: EventLanes<SL, C> + 'static,
    C: Clone + Send + 'static,
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

    /// Advances the internal current time to `time` if it is newer.
    ///
    /// Calling this with a time older than or equal to the current time is a no-op.
    pub fn advance_to(&self, time: i64) -> Result<(), ContimeError> {
        Ok(self.router.advance_to(time)?)
    }

    /// Returns the latest internal current time observed by this `contime`.
    pub fn current_time(&self) -> i64 {
        self.router.current_time()
    }

    /// Returns a clone of the apply context attached to this `contime`.
    ///
    /// Callers that provide a custom apply wrapper can use this to inspect
    /// context-owned state after public `contime` operations complete.
    pub fn apply_context(&self) -> C {
        self.apply_context.clone()
    }

    /// Applies an event synchronously and waits for all affected workers to finish.
    pub fn apply_event<E: Event>(&self, event: E) -> Result<(), ContimeError>
    where
        EL: From<E>,
    {
        self.router.apply_event(event.into())?;
        Ok(())
    }

    /// Returns snapshot lanes for many snapshot ids at the same query time.
    ///
    /// Events at exactly `time` are included.
    ///
    /// Results are returned in the same order as `snapshot_ids`. Missing histories yield `None`.
    pub fn query_at(&self, time: i64, snapshot_ids: &[u128]) -> Result<Vec<Option<SL>>, ContimeError> {
        Ok(self.router.query_at(time, snapshot_ids)?)
    }
}
