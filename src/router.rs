use std::marker::PhantomData;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ahash::RandomState;
use crossbeam_channel::bounded;
use flume::bounded as flume_bounded;

use crate::handle::{
    AdvanceHandle, ApplyHandle, EventQueryHandle, QueryEventsResult, QueryHandle, QueryResult, TimeAdvance, TimeAdvanceSubscription,
    WakeSubscription,
};
use crate::{EventLanes, SnapshotLanes, Worker, WorkerInbound};

const SUBSCRIPTION_CHANNEL_CAPACITY: usize = 1_000_000;

#[derive(Debug)]
pub enum RouterError {
    MemoryFull,
    Error,
}

pub struct Router<SL: SnapshotLanes<Event = EL>, EL: EventLanes<SL, C>, C = ()> {
    hasher: RandomState,
    workers: Vec<Worker<SL, EL, C>>,
    memory_budget: Arc<AtomicU64>,
    memory_usage: Arc<AtomicU64>,
    next_wake_subscription_id: AtomicU64,
    current_time: Arc<AtomicI64>,
    time_advance_subscribers: Arc<Mutex<Vec<flume::Sender<TimeAdvance>>>>,
    _context: PhantomData<C>,
}

impl<SL, EL> Router<SL, EL, ()>
where
    SL: SnapshotLanes<Event = EL> + 'static,
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
    SL: SnapshotLanes<Event = EL> + 'static,
    EL: EventLanes<SL, C> + 'static,
    C: Clone + Send + 'static,
{
    pub fn new_with_apply_context(worker_count: usize, memory_budget_bytes: u64, apply_context: C) -> Self {
        let hasher = RandomState::new();

        let memory_budget = Arc::new(AtomicU64::new(memory_budget_bytes));
        let memory_usage = Arc::new(AtomicU64::new(0));

        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            workers.push(Worker::<SL, EL, C>::new(Arc::clone(&memory_usage), apply_context.clone()));
        }

        Self {
            hasher,
            workers,
            memory_budget,
            memory_usage,
            next_wake_subscription_id: AtomicU64::new(1),
            current_time: Arc::new(AtomicI64::new(0)),
            time_advance_subscribers: Arc::new(Mutex::new(Vec::new())),
            _context: PhantomData,
        }
    }

    pub fn with_history_horizon_and_apply_context(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: i64,
        apply_context: C,
    ) -> Self {
        let hasher = RandomState::new();

        let memory_budget = Arc::new(AtomicU64::new(memory_budget_bytes));
        let memory_usage = Arc::new(AtomicU64::new(0));

        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            workers.push(Worker::<SL, EL, C>::with_history_horizon(
                Arc::clone(&memory_usage),
                lower_time_horizon_delta,
                apply_context.clone(),
            ));
        }

        Self {
            hasher,
            workers,
            memory_budget,
            memory_usage,
            next_wake_subscription_id: AtomicU64::new(1),
            current_time: Arc::new(AtomicI64::new(0)),
            time_advance_subscribers: Arc::new(Mutex::new(Vec::new())),
            _context: PhantomData,
        }
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

    pub fn many_at(&self, time: i64, snapshot_ids: &[u128]) -> Result<Vec<Option<SL>>, RouterError> {
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

    pub fn snapshot_lanes_by_worker(&self, time: i64) -> Result<Vec<Vec<SL>>, RouterError> {
        let mut rxs = Vec::with_capacity(self.workers.len());
        for worker in &self.workers {
            let (tx, rx) = bounded(1);
            worker.worker_inbound_tx.send(WorkerInbound::SnapshotLanesAt { time, reply: tx }).map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        let mut lanes_by_worker = Vec::with_capacity(rxs.len());
        for rx in rxs {
            lanes_by_worker.push(rx.recv().map_err(|_| RouterError::Error)?);
        }

        Ok(lanes_by_worker)
    }

    pub fn query_events_between(&self, snapshot_id: u128, from_time: i64, to_time: i64) -> Result<EventQueryHandle<EL>, RouterError> {
        let index = self.worker_index(snapshot_id);
        let (tx, rx) = bounded(1);
        self.workers[index]
            .worker_inbound_tx
            .send(WorkerInbound::EventsBetween { snapshot_id, from_time, to_time, reply: tx })
            .map_err(|_| RouterError::Error)?;

        Ok(EventQueryHandle::new(rx))
    }

    pub fn events_between(&self, snapshot_id: u128, from_time: i64, to_time: i64) -> Result<QueryEventsResult<EL>, RouterError> {
        self.query_events_between(snapshot_id, from_time, to_time)?.wait().map_err(|_| RouterError::Error)
    }

    pub fn send_advance(&self, time: i64) -> Result<AdvanceHandle, RouterError> {
        let current_time = Arc::clone(&self.current_time);
        let subscribers = Arc::clone(&self.time_advance_subscribers);
        let mut rxs = Vec::new();
        for worker in &self.workers {
            let (tx, rx) = bounded(1);
            worker.worker_inbound_tx.send(WorkerInbound::AdvanceTime { time, reply: tx }).map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        Ok(AdvanceHandle::new_with_completion(
            rxs,
            Box::new(move || {
                let advanced_to = current_time.fetch_add(time, Ordering::Relaxed).saturating_add(time);
                notify_time_advances(&subscribers, TimeAdvance { current_time: advanced_to, delta: time });
            }),
        ))
    }

    pub fn advance(&self, time: i64) -> Result<(), RouterError> {
        self.send_advance(time)?.wait().map_err(|_| RouterError::Error)
    }

    pub fn send_advance_to(&self, time: i64) -> Result<AdvanceHandle, RouterError> {
        let current = self.current_time.load(Ordering::Relaxed);
        let delta = time.saturating_sub(current);
        if delta <= 0 {
            return Ok(AdvanceHandle::new(Vec::new()));
        }
        self.send_advance(delta)
    }

    pub fn advance_to(&self, time: i64) -> Result<(), RouterError> {
        self.send_advance_to(time)?.wait().map_err(|_| RouterError::Error)
    }

    pub fn current_time(&self) -> i64 {
        self.current_time.load(Ordering::Relaxed)
    }

    pub fn subscribe_wakes(&self) -> Result<WakeSubscription<SL>, RouterError> {
        let subscription_id = self.next_wake_subscription_id.fetch_add(1, Ordering::Relaxed);
        let (wake_tx, wake_rx) = flume_bounded(SUBSCRIPTION_CHANNEL_CAPACITY);
        let mut worker_txs = Vec::with_capacity(self.workers.len());

        for worker in &self.workers {
            let (tx, rx) = bounded(1);
            worker_txs.push(worker.worker_inbound_tx.clone());
            worker
                .worker_inbound_tx
                .send(WorkerInbound::SubscribeWakes { subscription_id, tx: wake_tx.clone(), reply: tx })
                .map_err(|_| RouterError::Error)?;
            rx.recv().map_err(|_| RouterError::Error)?;
        }

        Ok(WakeSubscription::new(
            wake_rx,
            Some(Box::new(move || {
                for worker_tx in worker_txs {
                    let _ = worker_tx.send(WorkerInbound::UnsubscribeWakes { subscription_id });
                }
            })),
        ))
    }

    pub fn subscribe_time_advances(&self) -> Result<TimeAdvanceSubscription, RouterError> {
        let (tx, rx) = flume_bounded(SUBSCRIPTION_CHANNEL_CAPACITY);
        self.time_advance_subscribers.lock().map_err(|_| RouterError::Error)?.push(tx);
        Ok(TimeAdvanceSubscription::new(rx))
    }

    fn worker_index(&self, snapshot_id: u128) -> usize {
        let hash = self.hasher.hash_one(snapshot_id);

        hash as usize % self.workers.len()
    }
}

fn notify_time_advances(subscribers: &Arc<Mutex<Vec<flume::Sender<TimeAdvance>>>>, wake: TimeAdvance) {
    if let Ok(mut subscribers) = subscribers.lock() {
        subscribers.retain(|tx| match tx.send(wake) {
            Ok(()) => true,
            Err(flume::SendError(_)) => false,
        });
    }
}
