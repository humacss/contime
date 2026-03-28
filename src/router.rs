use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ahash::RandomState;
use flume::bounded;

use crate::handle::{AdvanceHandle, ApplyHandle, QueryHandle, QueryResult};
use crate::{EventLanes, SnapshotLanes, Worker, WorkerInbound};

#[derive(Debug)]
pub enum RouterError {
    MemoryFull,
    Error,
}

pub struct Router<SL: SnapshotLanes<Event = EL>, EL: EventLanes<SL>> {
    hasher: RandomState,
    workers: Vec<Worker<SL, EL>>,
    memory_budget: Arc<AtomicU64>,
    memory_usage: Arc<AtomicU64>,
}

impl<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL> + 'static> Router<SL, EL> {
    pub fn new(worker_count: usize, memory_budget_bytes: u64) -> Self {
        let hasher = RandomState::new();

        let memory_budget = Arc::new(AtomicU64::new(memory_budget_bytes));
        let memory_usage = Arc::new(AtomicU64::new(0));

        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            workers.push(Worker::new(Arc::clone(&memory_usage)));
        }

        Self { hasher, workers, memory_budget, memory_usage }
    }

    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: i64) -> Self {
        let hasher = RandomState::new();

        let memory_budget = Arc::new(AtomicU64::new(memory_budget_bytes));
        let memory_usage = Arc::new(AtomicU64::new(0));

        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            workers.push(Worker::with_history_horizon(Arc::clone(&memory_usage), lower_time_horizon_delta));
        }

        Self { hasher, workers, memory_budget, memory_usage }
    }

    pub fn send_event(&self, event_lane: EL) -> Result<ApplyHandle, RouterError> {
        let usage = self.memory_usage.load(Ordering::Relaxed);
        let budget = self.memory_budget.load(Ordering::Relaxed);
        if usage + event_lane.conservative_size() >= budget {
            return Err(RouterError::MemoryFull);
        }

        let mut rxs = Vec::new();
        for snapshot in event_lane.snapshots() {
            let snapshot_id = snapshot.id();
            let index = self.worker_index(snapshot_id);
            let (tx, rx) = bounded(1);
            self.workers[index]
                .worker_inbound_tx
                .send(WorkerInbound::Event { snapshot_id, event: event_lane.clone(), initial_snapshot: snapshot, reply: tx })
                .map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        Ok(ApplyHandle::new(rxs))
    }

    pub fn apply_event(&self, event_lane: EL) -> Result<(), RouterError> {
        self.send_event(event_lane)?.wait().map_err(|_| RouterError::Error)
    }

    pub fn send_snapshot(&self, snapshot_lane: SL) -> Result<ApplyHandle, RouterError> {
        let usage = self.memory_usage.load(Ordering::Relaxed);
        let budget = self.memory_budget.load(Ordering::Relaxed);
        if usage + snapshot_lane.conservative_size() >= budget {
            return Err(RouterError::MemoryFull);
        }

        let index = self.worker_index(snapshot_lane.id());
        let (tx, rx) = bounded(1);
        self.workers[index]
            .worker_inbound_tx
            .send(WorkerInbound::Snapshot { snapshot: snapshot_lane, reply: tx })
            .map_err(|_| RouterError::Error)?;

        Ok(ApplyHandle::new(vec![rx]))
    }

    pub fn apply_snapshot(&self, snapshot_lane: SL) -> Result<(), RouterError> {
        self.send_snapshot(snapshot_lane)?.wait().map_err(|_| RouterError::Error)
    }

    pub fn query_at(&self, time: i64, snapshot_id: u128) -> Result<QueryHandle<SL>, RouterError> {
        let index = self.worker_index(snapshot_id);
        let (tx, rx) = bounded(1);
        self.workers[index]
            .worker_inbound_tx
            .send(WorkerInbound::SnapshotAt { snapshot_id, time, reply: tx })
            .map_err(|_| RouterError::Error)?;

        Ok(QueryHandle::new(rx))
    }

    pub fn at(&self, time: i64, snapshot_id: u128) -> Result<QueryResult<SL>, RouterError> {
        self.query_at(time, snapshot_id)?.wait().map_err(|_| RouterError::Error)
    }

    pub fn send_advance(&self, time: i64) -> Result<AdvanceHandle, RouterError> {
        let mut rxs = Vec::new();
        for worker in &self.workers {
            let (tx, rx) = bounded(1);
            worker.worker_inbound_tx.send(WorkerInbound::AdvanceTime { time, reply: tx }).map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        Ok(AdvanceHandle::new(rxs))
    }

    pub fn advance(&self, time: i64) -> Result<(), RouterError> {
        self.send_advance(time)?.wait().map_err(|_| RouterError::Error)
    }

    fn worker_index(&self, snapshot_id: u128) -> usize {
        let hash = self.hasher.hash_one(snapshot_id);

        hash as usize % self.workers.len()
    }
}
