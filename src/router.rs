use std::collections::{BTreeMap, HashSet};
use std::marker::PhantomData;
use std::ops::{Bound, RangeBounds};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use ahash::RandomState;
use crossbeam_channel::{bounded, unbounded, Sender};

use crate::api::merge_event_rejections;
use crate::worker::{Completion, WorkerInput};
use crate::{
    ApplyWrapper, ContimeKey, ContimeTime, EventRejection, EventRejectionReason, Input, InputJournalEntry, InputLanes, SnapshotLanes,
    Worker, WorkerInbound,
};

type RoutedWorkerInputs<IL> = Vec<Vec<WorkerInput<IL>>>;
type RoutedInputsResult<IL, T> = Result<(RoutedWorkerInputs<IL>, Vec<EventRejection>), RouterError<T>>;

struct CanonicalInputIndex<T> {
    retained_ids: HashSet<u128>,
    ids_by_retention_time: BTreeMap<T, Vec<u128>>,
}

impl<T> Default for CanonicalInputIndex<T> {
    fn default() -> Self {
        Self { retained_ids: HashSet::new(), ids_by_retention_time: BTreeMap::new() }
    }
}

impl<T> CanonicalInputIndex<T>
where
    T: ContimeTime,
{
    fn contains(&self, input_id: u128) -> bool {
        self.retained_ids.contains(&input_id)
    }

    fn insert(&mut self, input_id: u128, time: T) {
        assert!(self.retained_ids.insert(input_id), "canonical input ID was inserted twice");
        self.ids_by_retention_time.entry(time).or_default().push(input_id);
    }

    fn prune_before(&mut self, earliest_time: T) -> usize {
        let retained = self.ids_by_retention_time.split_off(&earliest_time);
        let removed = std::mem::replace(&mut self.ids_by_retention_time, retained);
        removed.into_values().flatten().filter(|input_id| self.retained_ids.remove(input_id)).count()
    }
}

const fn canonical_input_index_entry_size<T>() -> u64 {
    // One ID in the identity set, one ID in its retention bucket, and a
    // conservative repeated time charge even when IDs share a bucket.
    (size_of::<u128>() * 2 + size_of::<T>()) as u64
}

#[derive(Debug)]
pub enum RouterError<T: ContimeTime> {
    MemoryFull,
    InputBeforeHistoryHorizon { input_time: T, earliest_time: T },
    Error,
}

pub struct Router<SL, IL, C = (), G = ()>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    hasher: RandomState,
    workers: Vec<Worker<SL, IL, C>>,
    memory_budget: Arc<AtomicU64>,
    memory_usage: Arc<AtomicU64>,
    current_time: Arc<RwLock<SL::Time>>,
    canonical_inputs: Mutex<CanonicalInputIndex<SL::Time>>,
    lower_time_horizon_delta: SL::Time,
    global_context: Arc<G>,
    _context: PhantomData<C>,
}

impl<SL, IL> Router<SL, IL, (), ()>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
{
    pub fn new(worker_count: usize, memory_budget_bytes: u64) -> Self {
        Self::new_with_apply_context(worker_count, memory_budget_bytes, ())
    }

    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: SL::Time) -> Self {
        Self::with_history_horizon_and_apply_context(worker_count, memory_budget_bytes, lower_time_horizon_delta, ())
    }
}

impl<SL, IL, C> Router<SL, IL, C, ()>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
    C: ApplyWrapper<SL> + Clone + Send + 'static,
{
    pub fn new_with_apply_context(worker_count: usize, memory_budget_bytes: u64, apply_context: C) -> Self {
        Self::new_with_contexts(worker_count, memory_budget_bytes, (), move |_, _| apply_context.clone())
    }

    pub fn with_history_horizon_and_apply_context(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: SL::Time,
        apply_context: C,
    ) -> Self {
        Self::with_history_horizon_and_contexts(worker_count, memory_budget_bytes, lower_time_horizon_delta, (), move |_, _| {
            apply_context.clone()
        })
    }
}

impl<SL, IL, C, G> Router<SL, IL, C, G>
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
        mut make_apply_context: F,
    ) -> Self
    where
        F: FnMut(usize, Arc<G>) -> C,
    {
        assert!(worker_count > 0, "worker_count must be greater than zero");

        let hasher = RandomState::new();
        let global_context = Arc::new(global_context);
        let memory_budget = Arc::new(AtomicU64::new(memory_budget_bytes));
        let memory_usage = Arc::new(AtomicU64::new(0));
        let mut worker_txs = Vec::<Sender<WorkerInbound<SL, IL>>>::with_capacity(worker_count);
        let mut worker_rxs = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let (tx, rx) = unbounded::<WorkerInbound<SL, IL>>();
            worker_txs.push(tx);
            worker_rxs.push(rx);
        }
        let worker_txs = Arc::new(worker_txs);

        let mut workers = Vec::with_capacity(worker_count);
        for (worker_index, rx) in worker_rxs.into_iter().enumerate() {
            workers.push(Worker::<SL, IL, C>::with_parts(
                worker_txs[worker_index].clone(),
                rx,
                worker_index,
                Arc::clone(&worker_txs),
                hasher.clone(),
                Arc::clone(&memory_usage),
                lower_time_horizon_delta.clone(),
                make_apply_context(worker_index, Arc::clone(&global_context)),
            ));
        }

        Self {
            hasher,
            workers,
            memory_budget,
            memory_usage,
            current_time: Arc::new(RwLock::new(SL::Time::default())),
            canonical_inputs: Mutex::new(CanonicalInputIndex::default()),
            lower_time_horizon_delta,
            global_context,
            _context: PhantomData,
        }
    }

    pub fn global_context(&self) -> Arc<G> {
        Arc::clone(&self.global_context)
    }

    pub fn apply<I>(&self, inputs: I) -> Result<Vec<EventRejection>, RouterError<SL::Time>>
    where
        I: IntoIterator<Item = IL>,
    {
        let (worker_inputs, rejections) = self.route_inputs(inputs)?;
        let mut replies = Vec::new();
        let mut merged = Vec::new();

        for (worker_index, inputs) in worker_inputs.into_iter().enumerate() {
            if inputs.is_empty() {
                continue;
            }
            let (tx, rx) = bounded(1);
            self.workers[worker_index]
                .worker_inbound_tx
                .send(WorkerInbound::Inputs { inputs, completion: Completion::Respond(tx) })
                .map_err(|_| RouterError::Error)?;
            replies.push(rx);
        }

        for reply in replies {
            let worker_rejections = reply.recv().map_err(|_| RouterError::Error)?;
            merge_event_rejections(&mut merged, worker_rejections);
        }
        merge_event_rejections(&mut merged, rejections);
        Ok(merged)
    }

    pub fn send<I>(&self, inputs: I) -> Result<(), RouterError<SL::Time>>
    where
        I: IntoIterator<Item = IL>,
    {
        let (worker_inputs, _rejections) = self.route_inputs(inputs)?;
        for (worker_index, inputs) in worker_inputs.into_iter().enumerate() {
            if inputs.is_empty() {
                continue;
            }
            self.workers[worker_index]
                .worker_inbound_tx
                .send(WorkerInbound::Inputs { inputs, completion: Completion::None })
                .map_err(|_| RouterError::Error)?;
        }
        Ok(())
    }

    fn route_inputs<I>(&self, inputs: I) -> RoutedInputsResult<IL, SL::Time>
    where
        I: IntoIterator<Item = IL>,
    {
        let mut worker_inputs = Vec::with_capacity(self.workers.len());
        worker_inputs.resize_with(self.workers.len(), Vec::new);

        let current_time = self.current_time.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let earliest_time = current_time.saturating_sub(self.lower_time_horizon_delta.clone());
        let mut canonical_inputs = self.canonical_inputs.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut accepted_ids = HashSet::new();
        let mut accepted_inputs = Vec::new();
        let mut rejections = Vec::new();
        let mut input_size = 0u64;
        let mut journal_size = 0u64;
        let mut routed_snapshots = Vec::<(u128, usize)>::new();

        for input in inputs {
            let input_id = Input::id(&input);
            if canonical_inputs.contains(input_id) || accepted_ids.contains(&input_id) {
                continue;
            }
            routed_snapshots.clear();
            input.visit_snapshot_ids(&mut |snapshot_id| routed_snapshots.push((snapshot_id, self.worker_index(snapshot_id))));
            if routed_snapshots.is_empty() {
                continue;
            }
            let input_time = Input::time(&input);
            if input_time < earliest_time {
                rejections.push(EventRejection::new(input_id, EventRejectionReason::BeforeHistoryHorizon));
                continue;
            }
            accepted_ids.insert(input_id);
            accepted_inputs.push((input_id, input_time));
            let lane_size = Input::conservative_size(&input);
            input_size = input_size.saturating_add(lane_size);
            journal_size = journal_size.saturating_add(canonical_input_index_entry_size::<SL::Time>());
            let mut routed_worker_indexes = Vec::new();
            for &(snapshot_id, index) in &routed_snapshots {
                if !routed_worker_indexes.contains(&index) {
                    routed_worker_indexes.push(index);
                    journal_size = journal_size.saturating_add(lane_size);
                }
                journal_size = journal_size.saturating_add(size_of::<u128>() as u64);
                worker_inputs[index].push(WorkerInput { snapshot_id, input: input.clone() });
            }
        }

        let usage = self.memory_usage.load(Ordering::Relaxed);
        let budget = self.memory_budget.load(Ordering::Relaxed);
        if usage.saturating_add(input_size).saturating_add(journal_size) >= budget {
            return Err(RouterError::MemoryFull);
        }

        for (input_id, time) in accepted_inputs {
            canonical_inputs.insert(input_id, time);
        }
        self.memory_usage
            .fetch_add((accepted_ids.len() as u64).saturating_mul(canonical_input_index_entry_size::<SL::Time>()), Ordering::Relaxed);

        Ok((worker_inputs, rejections))
    }

    pub fn inspect_inputs<R>(&self, range: R) -> Result<Vec<InputJournalEntry<IL>>, RouterError<SL::Time>>
    where
        R: RangeBounds<SL::Time>,
    {
        let start = owned_bound(range.start_bound());
        let end = owned_bound(range.end_bound());
        let mut replies = Vec::with_capacity(self.workers.len());

        for worker in &self.workers {
            let (tx, rx) = bounded(1);
            worker
                .worker_inbound_tx
                .send(WorkerInbound::InputsInRange { start: start.clone(), end: end.clone(), reply: tx })
                .map_err(|_| RouterError::Error)?;
            replies.push(rx);
        }

        let mut merged = Vec::<InputJournalEntry<IL>>::new();
        for reply in replies {
            for entry in reply.recv().map_err(|_| RouterError::Error)? {
                let key = ContimeKey::from_input(&entry.input);
                match merged.binary_search_by_key(&key, |entry| ContimeKey::from_input(&entry.input)) {
                    Ok(index) => merge_snapshot_ids(&mut merged[index].routed_snapshot_ids, entry.routed_snapshot_ids),
                    Err(index) => merged.insert(index, entry),
                }
            }
        }
        Ok(merged)
    }

    pub fn query_at(&self, time: SL::Time, snapshot_ids: &[u128]) -> Result<Vec<Option<SL>>, RouterError<SL::Time>> {
        if snapshot_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut requests_by_worker = Vec::with_capacity(self.workers.len());
        requests_by_worker.resize_with(self.workers.len(), Vec::new);
        for (position, snapshot_id) in snapshot_ids.iter().copied().enumerate() {
            requests_by_worker[self.worker_index(snapshot_id)].push((position, snapshot_id));
        }

        let mut replies = Vec::new();
        for (worker_index, snapshot_requests) in requests_by_worker.into_iter().enumerate() {
            if snapshot_requests.is_empty() {
                continue;
            }
            let (tx, rx) = bounded(1);
            self.workers[worker_index]
                .worker_inbound_tx
                .send(WorkerInbound::SnapshotsAt { snapshot_requests, time: time.clone(), reply: tx })
                .map_err(|_| RouterError::Error)?;
            replies.push(rx);
        }

        let mut results = Vec::with_capacity(snapshot_ids.len());
        results.resize_with(snapshot_ids.len(), || None);
        for reply in replies {
            for (position, snapshot_lane) in reply.recv().map_err(|_| RouterError::Error)? {
                results[position] = snapshot_lane;
            }
        }
        Ok(results)
    }

    pub fn advance_to(&self, time: SL::Time) -> Result<(), RouterError<SL::Time>> {
        {
            let mut current = self.current_time.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            if time <= *current {
                return Ok(());
            }
            *current = time.clone();
        }

        let mut replies = Vec::new();
        for worker in &self.workers {
            let (tx, rx) = bounded(1);
            worker.worker_inbound_tx.send(WorkerInbound::AdvanceTime { time: time.clone(), reply: tx }).map_err(|_| RouterError::Error)?;
            replies.push(rx);
        }
        for reply in replies {
            reply.recv().map_err(|_| RouterError::Error)?;
        }
        let earliest_time = time.saturating_sub(self.lower_time_horizon_delta.clone());
        let removed_ids = self.canonical_inputs.lock().unwrap_or_else(std::sync::PoisonError::into_inner).prune_before(earliest_time);
        let removed_bytes = (removed_ids as u64).saturating_mul(canonical_input_index_entry_size::<SL::Time>());
        let _ = self.memory_usage.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |usage| Some(usage.saturating_sub(removed_bytes)));
        Ok(())
    }

    pub fn current_time(&self) -> SL::Time {
        self.current_time.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    fn worker_index(&self, snapshot_id: u128) -> usize {
        self.hasher.hash_one(snapshot_id) as usize % self.workers.len()
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
