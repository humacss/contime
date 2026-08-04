use std::marker::PhantomData;
use std::ops::{Bound, RangeBounds};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use ahash::RandomState;
use crossbeam_channel::{bounded, unbounded, Sender};

use crate::worker::WorkerEvent;
use crate::{
    ApplyError, ApplyEvents, ApplyWrapper, ContimeKey, ContimeTime, EventJournalEntry, EventLanes, SnapshotLanes, Worker, WorkerInbound,
};

#[derive(Debug)]
pub enum RouterError<T: ContimeTime> {
    MemoryFull,
    EventBeforeHistoryHorizon { event_time: T, earliest_time: T },
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
    current_time: Arc<RwLock<SL::Time>>,
    lower_time_horizon_delta: SL::Time,
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

    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: SL::Time) -> Self {
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
        lower_time_horizon_delta: SL::Time,
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
            lower_time_horizon_delta,
            global_context,
            _context: PhantomData,
        }
    }

    pub fn global_context(&self) -> Arc<G> {
        Arc::clone(&self.global_context)
    }

    pub fn apply_events<I>(&self, event_lanes: I) -> Result<(), RouterError<SL::Time>>
    where
        I: IntoIterator<Item = EL>,
    {
        let worker_events = self.route_events(event_lanes)?;
        let mut rxs = Vec::new();

        for (worker_index, events) in worker_events.into_iter().enumerate() {
            if events.is_empty() {
                continue;
            }

            let (tx, rx) = bounded(1);
            self.workers[worker_index]
                .worker_inbound_tx
                .send(WorkerInbound::Events { events, reply: tx })
                .map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        for rx in rxs {
            rx.recv().map_err(|_| RouterError::Error)?.map_err(RouterError::ApplyFailed)?;
        }
        Ok(())
    }

    pub fn send_events<I>(&self, event_lanes: I) -> Result<(), RouterError<SL::Time>>
    where
        I: IntoIterator<Item = EL>,
    {
        let worker_events = self.route_events(event_lanes)?;

        for (worker_index, events) in worker_events.into_iter().enumerate() {
            if events.is_empty() {
                continue;
            }

            let (tx, _rx) = bounded(1);
            self.workers[worker_index]
                .worker_inbound_tx
                .send(WorkerInbound::Events { events, reply: tx })
                .map_err(|_| RouterError::Error)?;
        }

        Ok(())
    }

    fn route_events<I>(&self, event_lanes: I) -> Result<Vec<Vec<WorkerEvent<SL, EL>>>, RouterError<SL::Time>>
    where
        I: IntoIterator<Item = EL>,
    {
        let mut worker_events = Vec::with_capacity(self.workers.len());
        worker_events.resize_with(self.workers.len(), Vec::new);

        let current_time = self.current_time.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let earliest_time = current_time.saturating_sub(self.lower_time_horizon_delta.clone());
        let mut event_size = 0u64;
        let mut journal_size = 0u64;
        for event_lane in event_lanes {
            let event_time = event_lane.time();
            if event_time < earliest_time {
                return Err(RouterError::EventBeforeHistoryHorizon { event_time, earliest_time });
            }
            let lane_size = event_lane.conservative_size();
            event_size = event_size.saturating_add(lane_size);
            let routed_snapshots = event_lane.routed_snapshots();
            let mut routed_worker_indexes = Vec::new();
            for routed in routed_snapshots {
                let snapshot_id = routed.snapshot_id;
                let index = self.worker_index(snapshot_id);
                if !routed_worker_indexes.contains(&index) {
                    routed_worker_indexes.push(index);
                    journal_size = journal_size.saturating_add(lane_size);
                }
                journal_size = journal_size.saturating_add(size_of::<u128>() as u64);
                worker_events[index].push(WorkerEvent {
                    snapshot_id,
                    event: event_lane.clone(),
                    initial_snapshot: routed.initial_snapshot,
                });
            }
        }

        let usage = self.memory_usage.load(Ordering::Relaxed);
        let budget = self.memory_budget.load(Ordering::Relaxed);
        if usage.saturating_add(event_size).saturating_add(journal_size) >= budget {
            return Err(RouterError::MemoryFull);
        }

        Ok(worker_events)
    }

    pub fn inspect_events<R>(&self, range: R) -> Result<Vec<EventJournalEntry<EL>>, RouterError<SL::Time>>
    where
        R: RangeBounds<SL::Time>,
    {
        let start = owned_bound(range.start_bound());
        let end = owned_bound(range.end_bound());
        let mut rxs = Vec::with_capacity(self.workers.len());

        for worker in &self.workers {
            let (tx, rx) = bounded(1);
            worker
                .worker_inbound_tx
                .send(WorkerInbound::EventsInRange { start: start.clone(), end: end.clone(), reply: tx })
                .map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        let mut merged = Vec::<EventJournalEntry<EL>>::new();
        for rx in rxs {
            for entry in rx.recv().map_err(|_| RouterError::Error)? {
                let key = ContimeKey::from_event(&entry.event);
                match merged.binary_search_by_key(&key, |entry| ContimeKey::from_event(&entry.event)) {
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
                .send(WorkerInbound::SnapshotsAt { snapshot_requests, time: time.clone(), reply: tx })
                .map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        let mut results = Vec::with_capacity(snapshot_ids.len());
        results.resize_with(snapshot_ids.len(), || None);

        for rx in rxs {
            let batch = rx.recv().map_err(|_| RouterError::Error)?.map_err(RouterError::ApplyFailed)?;
            for (position, snapshot_lane) in batch {
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

        let mut rxs = Vec::new();
        for worker in &self.workers {
            let (tx, rx) = bounded(1);
            worker.worker_inbound_tx.send(WorkerInbound::AdvanceTime { time: time.clone(), reply: tx }).map_err(|_| RouterError::Error)?;
            rxs.push(rx);
        }

        for rx in rxs {
            rx.recv().map_err(|_| RouterError::Error)?.map_err(RouterError::ApplyFailed)?;
        }
        Ok(())
    }

    pub fn current_time(&self) -> SL::Time {
        self.current_time.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    fn worker_index(&self, snapshot_id: u128) -> usize {
        let hash = self.hasher.hash_one(snapshot_id);

        hash as usize % self.workers.len()
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
