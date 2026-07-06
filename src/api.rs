use std::marker::PhantomData;
use std::sync::Arc;

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
/// `SL` and `EL` are usually generated with [`crate::lanes!`].
pub struct Contime<SL: SnapshotLanes<Event = EL> + ApplyEvents, EL: EventLanes<SL, C>, C = (), G = ()>
where
    C: ApplyWrapper<SL>,
    C::Error: Into<ApplyError>,
{
    router: Router<SL, EL, C, G>,
    apply_context: Option<C>,
    global_context: Arc<G>,
    _context: PhantomData<C>,
}

impl<SL, EL> Contime<SL, EL, ()>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    (): ApplyWrapper<SL>,
    <() as ApplyWrapper<SL>>::Error: Into<ApplyError>,
    EL: EventLanes<SL> + 'static,
{
    /// Creates a `contime` instance with `worker_count` workers and a shared memory budget.
    ///
    /// Most users generate `SL` and `EL` with [`crate::lanes!`].
    pub fn new(worker_count: usize, memory_budget_bytes: u64) -> Self {
        let router = Router::<SL, EL>::new(worker_count, memory_budget_bytes);

        Self { router, apply_context: Some(()), global_context: Arc::new(()), _context: PhantomData }
    }

    /// Creates a `contime` instance that retains a bounded amount of history behind the
    /// internally advanced current time.
    ///
    /// Call [`Contime::advance`] to move the current time forward. History older than
    /// `current_time - lower_time_horizon_delta` becomes eligible for pruning.
    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: i64) -> Self {
        let router = Router::<SL, EL>::with_history_horizon(worker_count, memory_budget_bytes, lower_time_horizon_delta);

        Self { router, apply_context: Some(()), global_context: Arc::new(()), _context: PhantomData }
    }
}

impl<SL, EL, C> Contime<SL, EL, C, ()>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    C: ApplyWrapper<SL> + 'static,
    C::Error: Into<ApplyError>,
    EL: EventLanes<SL, C> + 'static,
    C: Clone + Send + 'static,
{
    /// Creates a `contime` instance with an explicit per-worker apply context.
    pub fn new_with_apply_context(worker_count: usize, memory_budget_bytes: u64, apply_context: C) -> Self {
        let router = Router::<SL, EL, C>::new_with_apply_context(worker_count, memory_budget_bytes, apply_context.clone());

        Self { router, apply_context: Some(apply_context), global_context: Arc::new(()), _context: PhantomData }
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

        Self { router, apply_context: Some(apply_context), global_context: Arc::new(()), _context: PhantomData }
    }

    /// Returns a clone of the apply context attached to this `contime`.
    ///
    /// This is available for clone-based contexts passed to
    /// [`Contime::new_with_apply_context`]. Contexts created through
    /// [`Contime::new_with_apply_context_factory`] live on their workers and do
    /// not have a single inspectable root context.
    pub fn apply_context(&self) -> C {
        self.apply_context.as_ref().expect("factory-created worker contexts do not expose a root apply context").clone()
    }
}

impl<SL, EL, C> Contime<SL, EL, C, ()>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    C: ApplyWrapper<SL> + Send + 'static,
    C::Error: Into<ApplyError>,
    EL: EventLanes<SL, C> + Send + 'static,
{
    /// Creates a `contime` instance with one apply context initialized per worker.
    pub fn new_with_apply_context_factory<F>(worker_count: usize, memory_budget_bytes: u64, mut make_apply_context: F) -> Self
    where
        F: FnMut(usize) -> C,
    {
        Self::new_with_contexts(worker_count, memory_budget_bytes, (), move |worker_id, _global_context| make_apply_context(worker_id))
    }

    /// Creates a history-bounded `contime` instance with one apply context initialized per worker.
    pub fn with_history_horizon_and_apply_context_factory<F>(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: i64,
        mut make_apply_context: F,
    ) -> Self
    where
        F: FnMut(usize) -> C,
    {
        Self::with_history_horizon_and_contexts(
            worker_count,
            memory_budget_bytes,
            lower_time_horizon_delta,
            (),
            move |worker_id, _global_context| make_apply_context(worker_id),
        )
    }
}

impl<SL, EL, C, G> Contime<SL, EL, C, G>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    C: ApplyWrapper<SL> + Send + 'static,
    C::Error: Into<ApplyError>,
    EL: EventLanes<SL, C> + Send + 'static,
    G: Send + Sync + 'static,
{
    /// Creates a `contime` instance with shared global context and one apply context
    /// initialized per worker.
    pub fn new_with_contexts<F>(worker_count: usize, memory_budget_bytes: u64, global_context: G, make_apply_context: F) -> Self
    where
        F: FnMut(usize, Arc<G>) -> C,
    {
        Self::with_history_horizon_and_contexts(worker_count, memory_budget_bytes, 0, global_context, make_apply_context)
    }

    /// Creates a history-bounded `contime` instance with shared global context and one
    /// apply context initialized per worker.
    pub fn with_history_horizon_and_contexts<F>(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: i64,
        global_context: G,
        make_apply_context: F,
    ) -> Self
    where
        F: FnMut(usize, Arc<G>) -> C,
    {
        let router = Router::<SL, EL, C, G>::with_history_horizon_and_contexts(
            worker_count,
            memory_budget_bytes,
            lower_time_horizon_delta,
            global_context,
            make_apply_context,
        );
        let global_context = router.global_context();

        Self { router, apply_context: None, global_context, _context: PhantomData }
    }

    /// Returns the shared global context attached to this `contime`.
    pub fn global_context(&self) -> Arc<G> {
        Arc::clone(&self.global_context)
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
