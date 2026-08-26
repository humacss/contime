use std::marker::PhantomData;
use std::ops::{Bound, RangeBounds};
use std::sync::{Arc, RwLock};

use crossbeam_channel::unbounded;

use crate::{ApplyWrapper, ContimeKey, ContimeTime, InputJournalEntry, InputLanes, Router, RouterError, SnapshotLanes};

/// One input that ConTime could not admit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventRejection {
    pub event_id: u128,
    pub reason: EventRejectionReason,
}

impl EventRejection {
    pub const fn new(event_id: u128, reason: EventRejectionReason) -> Self {
        Self { event_id, reason }
    }
}

/// Reason one input was rejected while the rest of its batch continued.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventRejectionReason {
    /// The input predates the earliest retained history time.
    BeforeHistoryHorizon,
    /// Retaining the input would exceed the configured memory budget.
    MemoryFull,
}

pub(crate) fn merge_event_rejections(target: &mut Vec<EventRejection>, incoming: Vec<EventRejection>) {
    if incoming.is_empty() {
        return;
    }
    target.extend(incoming);
    target.sort_unstable();
    target.dedup();
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

        let mut rejections = Vec::new();
        for _ in 0..expected {
            let worker_rejections = response_rx.recv().map_err(|_| ContimeError::ResponseDisconnected)?;
            merge_event_rejections(&mut rejections, worker_rejections);
        }
        Ok(rejections)
    }

    /// Enqueues temporal inputs without waiting for replay to finish.
    pub fn send<I>(&self, inputs: I) -> Result<(), ContimeError>
    where
        I: IntoIterator<Item = IL>,
    {
        self.router.dispatch_inputs(inputs, None)?;
        Ok(())
    }

    /// Returns retained canonical temporal inputs whose timestamps are within `range`.
    pub fn inspect_inputs<R>(&self, range: R) -> Result<Vec<InputJournalEntry<IL>>, ContimeError>
    where
        R: RangeBounds<SL::Time>,
    {
        let start = owned_bound(range.start_bound());
        let end = owned_bound(range.end_bound());
        let (response_tx, response_rx) = unbounded();
        let expected = self.router.dispatch_inspection(start, end, &response_tx)?;
        drop(response_tx);

        let mut merged = Vec::<InputJournalEntry<IL>>::new();
        for _ in 0..expected {
            for entry in response_rx.recv().map_err(|_| ContimeError::ResponseDisconnected)? {
                let key = ContimeKey::from_input(&entry.input);
                match merged.binary_search_by_key(&key, |entry| ContimeKey::from_input(&entry.input)) {
                    Ok(index) => merge_snapshot_ids(&mut merged[index].routed_snapshot_ids, entry.routed_snapshot_ids),
                    Err(index) => merged.insert(index, entry),
                }
            }
        }
        Ok(merged)
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

fn owned_bound<T: ContimeTime>(bound: Bound<&T>) -> Bound<T> {
    match bound {
        Bound::Included(value) => Bound::Included(value.clone()),
        Bound::Excluded(value) => Bound::Excluded(value.clone()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn merge_snapshot_ids(existing: &mut Vec<u128>, incoming: Vec<u128>) {
    for snapshot_id in incoming {
        if let Err(index) = existing.binary_search(&snapshot_id) {
            existing.insert(index, snapshot_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_event_rejections, EventRejection, EventRejectionReason};

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
