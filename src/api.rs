use std::marker::PhantomData;
use std::ops::RangeBounds;
use std::sync::Arc;

use crate::{ApplyWrapper, ContimeTime, InputJournalEntry, InputLanes, Router, RouterError, SnapshotLanes};

/// Errors returned by [`Contime`] operations.
#[derive(Debug)]
pub enum ContimeError<T: ContimeTime> {
    /// Applying the input would exceed the configured memory budget.
    MemoryFull,
    /// The temporal input predates the earliest time retained by this instance.
    InputBeforeHistoryHorizon { input_time: T, earliest_time: T },
    /// The requested snapshot id has no known history.
    NotFound,
    /// Internal routing error.
    RouterError(RouterError<T>),
}

impl<T: ContimeTime> From<RouterError<T>> for ContimeError<T> {
    fn from(error: RouterError<T>) -> Self {
        match error {
            RouterError::MemoryFull => Self::MemoryFull,
            RouterError::InputBeforeHistoryHorizon { input_time, earliest_time } => {
                Self::InputBeforeHistoryHorizon { input_time, earliest_time }
            }
            other => Self::RouterError(other),
        }
    }
}

/// Main entry point for building and querying continuous-time state.
///
/// `SL` and `IL` are usually generated with [`crate::lanes!`].
pub struct Contime<SL, IL, C = (), G = ()>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    router: Router<SL, IL, C, G>,
    apply_context: Option<C>,
    global_context: Arc<G>,
    _context: PhantomData<C>,
}

impl<SL, IL> Contime<SL, IL, (), ()>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
{
    pub fn new(worker_count: usize, memory_budget_bytes: u64) -> Self {
        let router = Router::<SL, IL>::new(worker_count, memory_budget_bytes);
        Self { router, apply_context: Some(()), global_context: Arc::new(()), _context: PhantomData }
    }

    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: SL::Time) -> Self {
        let router = Router::<SL, IL>::with_history_horizon(worker_count, memory_budget_bytes, lower_time_horizon_delta);
        Self { router, apply_context: Some(()), global_context: Arc::new(()), _context: PhantomData }
    }
}

impl<SL, IL, C> Contime<SL, IL, C, ()>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
    C: ApplyWrapper<SL> + Clone + Send + 'static,
{
    pub fn new_with_apply_context(worker_count: usize, memory_budget_bytes: u64, apply_context: C) -> Self {
        let router = Router::<SL, IL, C>::new_with_apply_context(worker_count, memory_budget_bytes, apply_context.clone());
        Self { router, apply_context: Some(apply_context), global_context: Arc::new(()), _context: PhantomData }
    }

    pub fn with_history_horizon_and_apply_context(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: SL::Time,
        apply_context: C,
    ) -> Self {
        let router = Router::<SL, IL, C>::with_history_horizon_and_apply_context(
            worker_count,
            memory_budget_bytes,
            lower_time_horizon_delta,
            apply_context.clone(),
        );
        Self { router, apply_context: Some(apply_context), global_context: Arc::new(()), _context: PhantomData }
    }

    pub fn apply_context(&self) -> C {
        self.apply_context.as_ref().expect("factory-created worker contexts do not expose a root apply context").clone()
    }
}

impl<SL, IL, C> Contime<SL, IL, C, ()>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
    C: ApplyWrapper<SL> + Send + 'static,
{
    pub fn new_with_apply_context_factory<F>(worker_count: usize, memory_budget_bytes: u64, mut make_apply_context: F) -> Self
    where
        F: FnMut(usize) -> C,
    {
        Self::new_with_contexts(worker_count, memory_budget_bytes, (), move |worker_id, _| make_apply_context(worker_id))
    }

    pub fn with_history_horizon_and_apply_context_factory<F>(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: SL::Time,
        mut make_apply_context: F,
    ) -> Self
    where
        F: FnMut(usize) -> C,
    {
        Self::with_history_horizon_and_contexts(worker_count, memory_budget_bytes, lower_time_horizon_delta, (), move |worker_id, _| {
            make_apply_context(worker_id)
        })
    }
}

impl<SL, IL, C, G> Contime<SL, IL, C, G>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
    C: ApplyWrapper<SL> + Send + 'static,
    G: Send + Sync + 'static,
{
    pub fn new_with_contexts<F>(worker_count: usize, memory_budget_bytes: u64, global_context: G, make_apply_context: F) -> Self
    where
        F: FnMut(usize, Arc<G>) -> C,
    {
        Self::with_history_horizon_and_contexts(worker_count, memory_budget_bytes, SL::Time::default(), global_context, make_apply_context)
    }

    pub fn with_history_horizon_and_contexts<F>(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: SL::Time,
        global_context: G,
        make_apply_context: F,
    ) -> Self
    where
        F: FnMut(usize, Arc<G>) -> C,
    {
        let router = Router::<SL, IL, C, G>::with_history_horizon_and_contexts(
            worker_count,
            memory_budget_bytes,
            lower_time_horizon_delta,
            global_context,
            make_apply_context,
        );
        let global_context = router.global_context();
        Self { router, apply_context: None, global_context, _context: PhantomData }
    }

    pub fn global_context(&self) -> Arc<G> {
        Arc::clone(&self.global_context)
    }

    pub fn advance_to(&self, time: SL::Time) -> Result<(), ContimeError<SL::Time>> {
        Ok(self.router.advance_to(time)?)
    }

    pub fn current_time(&self) -> SL::Time {
        self.router.current_time()
    }

    /// Applies temporal inputs synchronously and waits for all affected workers.
    pub fn apply<I>(&self, inputs: I) -> Result<(), ContimeError<SL::Time>>
    where
        I: IntoIterator<Item = IL>,
    {
        Ok(self.router.apply(inputs)?)
    }

    /// Enqueues temporal inputs without waiting for replay to finish.
    pub fn send<I>(&self, inputs: I) -> Result<(), ContimeError<SL::Time>>
    where
        I: IntoIterator<Item = IL>,
    {
        Ok(self.router.send(inputs)?)
    }

    /// Returns retained canonical temporal inputs whose timestamps are within `range`.
    pub fn inspect_inputs<R>(&self, range: R) -> Result<Vec<InputJournalEntry<IL>>, ContimeError<SL::Time>>
    where
        R: RangeBounds<SL::Time>,
    {
        Ok(self.router.inspect_inputs(range)?)
    }

    pub fn query_at(&self, time: SL::Time, snapshot_ids: &[u128]) -> Result<Vec<Option<SL>>, ContimeError<SL::Time>> {
        Ok(self.router.query_at(time, snapshot_ids)?)
    }
}
