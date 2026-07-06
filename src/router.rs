use std::marker::PhantomData;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use ahash::RandomState;
use crossbeam_channel::{bounded, unbounded, Sender};

use crate::{ApplyError, ApplyEvents, ApplyWrapper, EventLanes, SnapshotLanes, Worker, WorkerInbound};

#[derive(Debug)]
pub enum RouterError {
    MemoryFull,
    ApplyFailed(ApplyError),
    Error,
}

pub struct Router<SL: SnapshotLanes<Event = EL> + ApplyEvents, EL: EventLanes<SL, C>, C = (), G = ()>
where
    C: ApplyWrapper<SL>,
    C::Error: Into<ApplyError>,
{
    hasher: RandomState,
    workers: Vec<Worker<SL, EL, C>>,
    memory_budget: Arc<AtomicU64>,
    memory_usage: Arc<AtomicU64>,
    current_time: Arc<AtomicI64>,
    global_context: Arc<G>,
    _context: PhantomData<C>,
}

impl<SL, EL> Router<SL, EL, ()>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    (): ApplyWrapper<SL>,
    <() as ApplyWrapper<SL>>::Error: Into<ApplyError>,
    EL: EventLanes<SL> + 'static,
{
    pub fn new(worker_count: usize, memory_budget_bytes: u64) -> Self {
        Self::new_with_apply_context(worker_count, memory_budget_bytes, ())
    }

    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: i64) -> Self {
        Self::with_history_horizon_and_apply_context(worker_count, memory_budget_bytes, lower_time_horizon_delta, ())
    }
}

impl<SL, EL, C> Router<SL, EL, C>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    C: ApplyWrapper<SL> + 'static,
    C::Error: Into<ApplyError>,
    EL: EventLanes<SL, C> + 'static,
    C: Clone + Send + 'static,
{
    pub fn new_with_apply_context(worker_count: usize, memory_budget_bytes: u64, apply_context: C) -> Self {
        Self::new_with_contexts(worker_count, memory_budget_bytes, (), move |_, _| apply_context.clone())
    }

    pub fn with_history_horizon_and_apply_context(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: i64,
        apply_context: C,
    ) -> Self {
        Self::with_history_horizon_and_contexts(worker_count, memory_budget_bytes, lower_time_horizon_delta, (), move |_, _| {
            apply_context.clone()
        })
    }
}

impl<SL, EL, C, G> Router<SL, EL, C, G>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    C: ApplyWrapper<SL> + Send + 'static,
    C::Error: Into<ApplyError>,
    EL: EventLanes<SL, C> + Send + 'static,
    G: Send + Sync + 'static,
{
    pub fn new_with_contexts<F>(worker_count: usize, memory_budget_bytes: u64, global_context: G, make_apply_context: F) -> Self
    where
        F: FnMut(usize, Arc<G>) -> C,
    {
        Self::with_history_horizon_and_contexts(worker_count, memory_budget_bytes, 0, global_context, make_apply_context)
    }

    pub fn with_history_horizon_and_contexts<F>(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: i64,
        global_context: G,
        mut make_apply_context: F,
    ) -> Self
    where
        F: FnMut(usize, Arc<G>) -> C,
    {
        let hasher = RandomState::new();

        let global_context = Arc::new(global_context);
        let memory_budget = Arc::new(AtomicU64::new(memory_budget_bytes));
        let memory_usage = Arc::new(AtomicU64::new(0));
        let mut worker_txs = Vec::<Sender<WorkerInbound<SL, EL>>>::with_capacity(worker_count);
        let mut worker_rxs = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let (tx, rx) = unbounded::<WorkerInbound<SL, EL>>();
            worker_txs.push(tx);
            worker_rxs.push(rx);
        }
        let worker_txs = Arc::new(worker_txs);

        let mut workers = Vec::with_capacity(worker_count);
        for (worker_index, rx) in worker_rxs.into_iter().enumerate() {
            workers.push(Worker::<SL, EL, C>::with_parts(
                worker_txs[worker_index].clone(),
                rx,
                worker_index,
                worker_txs.clone(),
                hasher.clone(),
                Arc::clone(&memory_usage),
                lower_time_horizon_delta,
                make_apply_context(worker_index, Arc::clone(&global_context)),
            ));
        }

        Self {
            hasher,
            workers,
            memory_budget,
            memory_usage,
            current_time: Arc::new(AtomicI64::new(0)),
            global_context,
            _context: PhantomData,
        }
    }

    pub fn global_context(&self) -> Arc<G> {
        Arc::clone(&self.global_context)
    }

    pub fn apply_event(&self, event_lane: EL) -> Result<(), RouterError> {
        let usage = self.memory_usage.load(Ordering::Relaxed);
        let budget = self.memory_budget.load(Ordering::Relaxed);
        if usage + event_lane.conservative_size() >= budget {
            return Err(RouterError::MemoryFull);
        }

        let mut rxs = Vec::new();
        for routed in event_lane.routed_snapshots() {
            let snapshot_id = routed.snapshot_id;
            let index = self.worker_index(snapshot_id);
            let (tx, rx) = bounded(1);
            self.workers[index]
                .worker_inbound_tx
                .send(WorkerInbound::Event { snapshot_id, event: event_lane.clone(), initial_snapshot: routed.initial_snapshot, reply: tx })
                .map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        for rx in rxs {
            rx.recv().map_err(|_| RouterError::Error)?.map_err(RouterError::ApplyFailed)?;
        }
        Ok(())
    }

    pub fn query_at(&self, time: i64, snapshot_ids: &[u128]) -> Result<Vec<Option<SL>>, RouterError> {
        if snapshot_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut requests_by_worker = Vec::with_capacity(self.workers.len());
        requests_by_worker.resize_with(self.workers.len(), Vec::new);

        for (position, snapshot_id) in snapshot_ids.iter().copied().enumerate() {
            let index = self.worker_index(snapshot_id);
            requests_by_worker[index].push((position, snapshot_id));
        }

        let mut rxs = Vec::new();
        for (worker_index, snapshot_requests) in requests_by_worker.into_iter().enumerate() {
            if snapshot_requests.is_empty() {
                continue;
            }

            let (tx, rx) = bounded(1);
            self.workers[worker_index]
                .worker_inbound_tx
                .send(WorkerInbound::SnapshotsAt { snapshot_requests, time, reply: tx })
                .map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        let mut results = Vec::with_capacity(snapshot_ids.len());
        results.resize_with(snapshot_ids.len(), || None);

        for rx in rxs {
            let batch = rx.recv().map_err(|_| RouterError::Error)?;
            for (position, snapshot_lane) in batch {
                results[position] = snapshot_lane;
            }
        }

        Ok(results)
    }

    pub fn advance_to(&self, time: i64) -> Result<(), RouterError> {
        let mut current = self.current_time.load(Ordering::Relaxed);
        loop {
            if time <= current {
                return Ok(());
            }
            match self.current_time.compare_exchange_weak(current, time, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }

        let mut rxs = Vec::new();
        for worker in &self.workers {
            let (tx, rx) = bounded(1);
            worker.worker_inbound_tx.send(WorkerInbound::AdvanceTime { time, reply: tx }).map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        for rx in rxs {
            rx.recv().map_err(|_| RouterError::Error)?;
        }
        Ok(())
    }

    pub fn current_time(&self) -> i64 {
        self.current_time.load(Ordering::Relaxed)
    }

    fn worker_index(&self, snapshot_id: u128) -> usize {
        let hash = self.hasher.hash_one(snapshot_id);

        hash as usize % self.workers.len()
    }
}
