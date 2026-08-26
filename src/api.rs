use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use crossbeam_channel::{unbounded, Receiver};

use crate::rejection::merge_event_rejections;
use crate::{ApplyWrapper, EventRejection, InputLanes, Router, RouterError, SnapshotLanes};

fn collect_event_rejections(response_rx: &Receiver<Vec<EventRejection>>, expected: usize) -> Result<Vec<EventRejection>, ContimeError> {
    let mut rejections = Vec::new();
    for _ in 0..expected {
        let worker_rejections = response_rx.recv().map_err(|_| ContimeError::ResponseDisconnected)?;
        merge_event_rejections(&mut rejections, worker_rejections);
    }
    Ok(rejections)
}

/// Benchmark-only access to request-channel completion aggregation.
#[doc(hidden)]
pub struct CompletionBenchmark;

impl CompletionBenchmark {
    pub fn run(worker_count: usize) -> usize {
        let (response_tx, response_rx) = unbounded();
        for _ in 0..worker_count {
            response_tx.send(Vec::new()).expect("benchmark response receiver remains connected");
        }
        drop(response_tx);
        collect_event_rejections(&response_rx, worker_count).expect("benchmark response count is exact").len()
    }
}

/// Errors returned by [`Contime`] operations.
#[derive(Debug)]
pub enum ContimeError {
    /// A worker stopped accepting operation messages.
    WorkerUnavailable,
    /// An affected worker exited without completing a synchronous request.
    ResponseDisconnected,
}

impl From<RouterError> for ContimeError {
    fn from(error: RouterError) -> Self {
        match error {
            RouterError::WorkerUnavailable => Self::WorkerUnavailable,
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
    current_time: RwLock<SL::Time>,
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
        Self {
            router,
            current_time: RwLock::new(SL::Time::default()),
            apply_context: Some(()),
            global_context: Arc::new(()),
            _context: PhantomData,
        }
    }

    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: SL::Time) -> Self {
        let router = Router::<SL, IL>::with_history_horizon(worker_count, memory_budget_bytes, lower_time_horizon_delta);
        Self {
            router,
            current_time: RwLock::new(SL::Time::default()),
            apply_context: Some(()),
            global_context: Arc::new(()),
            _context: PhantomData,
        }
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
        Self {
            router,
            current_time: RwLock::new(SL::Time::default()),
            apply_context: Some(apply_context),
            global_context: Arc::new(()),
            _context: PhantomData,
        }
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
        Self {
            router,
            current_time: RwLock::new(SL::Time::default()),
            apply_context: Some(apply_context),
            global_context: Arc::new(()),
            _context: PhantomData,
        }
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
        Self { router, current_time: RwLock::new(SL::Time::default()), apply_context: None, global_context, _context: PhantomData }
    }

    pub fn global_context(&self) -> Arc<G> {
        Arc::clone(&self.global_context)
    }

    pub fn advance_to(&self, time: SL::Time) -> Result<(), ContimeError> {
        {
            let mut current = self.current_time.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            if time <= *current {
                return Ok(());
            }
            *current = time.clone();
        }

        let (response_tx, response_rx) = unbounded();
        let expected = self.router.dispatch_advance(time, &response_tx)?;
        drop(response_tx);
        for _ in 0..expected {
            response_rx.recv().map_err(|_| ContimeError::ResponseDisconnected)?;
        }
        Ok(())
    }

    pub fn current_time(&self) -> SL::Time {
        self.current_time.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// Applies temporal inputs synchronously and waits for all affected workers.
    pub fn apply<I>(&self, inputs: I) -> Result<Vec<EventRejection>, ContimeError>
    where
        I: IntoIterator<Item = IL>,
    {
        let (response_tx, response_rx) = unbounded();
        let expected = self.router.dispatch_inputs(inputs, Some(&response_tx))?;
        drop(response_tx);

        collect_event_rejections(&response_rx, expected)
    }

    /// Enqueues temporal inputs without waiting for replay to finish.
    pub fn send<I>(&self, inputs: I) -> Result<(), ContimeError>
    where
        I: IntoIterator<Item = IL>,
    {
        self.router.dispatch_inputs(inputs, None)?;
        Ok(())
    }

    pub fn query_at(&self, time: SL::Time, snapshot_ids: &[u128]) -> Result<Vec<Option<SL>>, ContimeError> {
        if snapshot_ids.is_empty() {
            return Ok(Vec::new());
        }

        let positioned_snapshot_ids = snapshot_ids.iter().copied().enumerate().collect::<Vec<_>>();
        let (response_tx, response_rx) = unbounded();
        let expected = self.router.dispatch_query(time, &positioned_snapshot_ids, &response_tx)?;
        drop(response_tx);

        let mut results = Vec::with_capacity(snapshot_ids.len());
        results.resize_with(snapshot_ids.len(), || None);
        for _ in 0..expected {
            for (position, snapshot_lane) in response_rx.recv().map_err(|_| ContimeError::ResponseDisconnected)? {
                results[position] = snapshot_lane;
            }
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use crate::rejection::merge_event_rejections;
    use crate::{EventRejection, EventRejectionReason};

    #[test]
    fn rejection_merge_deduplicates_only_identical_event_and_reason_pairs() {
        let mut merged = vec![
            EventRejection::new(7, EventRejectionReason::MemoryFull),
            EventRejection::new(7, EventRejectionReason::BeforeHistoryHorizon),
        ];
        merge_event_rejections(
            &mut merged,
            vec![EventRejection::new(7, EventRejectionReason::MemoryFull), EventRejection::new(9, EventRejectionReason::MemoryFull)],
        );

        assert_eq!(
            merged,
            vec![
                EventRejection::new(7, EventRejectionReason::BeforeHistoryHorizon),
                EventRejection::new(7, EventRejectionReason::MemoryFull),
                EventRejection::new(9, EventRejectionReason::MemoryFull),
            ]
        );
    }

    #[test]
    fn empty_rejection_vector_is_the_success_value() {
        let mut merged = Vec::new();
        merge_event_rejections(&mut merged, Vec::new());
        assert!(merged.is_empty());
        assert_eq!(merged.capacity(), 0);
    }
}
