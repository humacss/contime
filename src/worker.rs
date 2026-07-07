use std::collections::hash_map::Entry;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ahash::{AHashMap, RandomState};
use crossbeam_channel::{Receiver, Sender};

use crate::{ApplyEvents, ApplyWrapper, EventLanes, SnapshotHistory, SnapshotLanes};

/// Error returned when a worker cannot apply an event batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    message: String,
}

impl ApplyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApplyError {}

impl From<std::convert::Infallible> for ApplyError {
    fn from(error: std::convert::Infallible) -> Self {
        match error {}
    }
}

pub type SnapshotId = u128;

pub struct WorkerEvent<SL, EL> {
    pub snapshot_id: u128,
    pub event: EL,
    pub initial_snapshot: SL,
}

pub enum WorkerInbound<SL: SnapshotLanes, EL> {
    Events { events: Vec<WorkerEvent<SL, EL>>, reply: Sender<Result<(), ApplyError>> },
    SnapshotsAt { snapshot_requests: Vec<(usize, u128)>, time: i64, reply: Sender<Vec<(usize, Option<SL>)>> },
    AdvanceTime { time: i64, reply: Sender<()> },
    Shutdown,
}

pub struct Worker<SL: SnapshotLanes + ApplyEvents, EL: EventLanes<SL, C>, C = ()> {
    pub worker_inbound_tx: Sender<WorkerInbound<SL, EL>>,

    threads: Vec<JoinHandle<()>>,
    is_running: Arc<AtomicBool>,
    _context: PhantomData<C>,
}

impl<SL: SnapshotLanes + ApplyEvents, EL: EventLanes<SL, C>, C> Drop for Worker<SL, EL, C> {
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);

        let _ = self.worker_inbound_tx.send(WorkerInbound::<SL, EL>::Shutdown);

        for thread in self.threads.drain(..) {
            if let Err(error) = thread.join() {
                eprintln!("contime worker thread panicked: {:?}", error);
            }
        }
    }
}

impl<SL, EL, C> Worker<SL, EL, C>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    EL: EventLanes<SL, C> + 'static + Send,
    C: ApplyWrapper<SL> + Send + 'static,
    C::Error: Into<ApplyError>,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_parts(
        worker_inbound_tx: Sender<WorkerInbound<SL, EL>>,
        worker_inbound_rx: Receiver<WorkerInbound<SL, EL>>,
        _worker_index: usize,
        _worker_txs: Arc<Vec<Sender<WorkerInbound<SL, EL>>>>,
        _hasher: RandomState,
        memory_usage: Arc<AtomicU64>,
        lower_time_horizon_delta: i64,
        apply_context: C,
    ) -> Self {
        let mut threads = Vec::with_capacity(1);
        let is_running = Arc::new(AtomicBool::new(true));

        {
            let is_running = Arc::clone(&is_running);
            let memory_usage = Arc::clone(&memory_usage);

            threads.push(thread::spawn(move || {
                handle_worker(is_running, worker_inbound_rx, memory_usage, lower_time_horizon_delta, apply_context);
            }));
        };

        Self { worker_inbound_tx, is_running, threads, _context: PhantomData }
    }
}

fn fetch_saturating_add_signed(atomic: &Arc<AtomicU64>, delta: i64, order: Ordering) {
    loop {
        let current = atomic.load(order);
        let new_val = if delta >= 0 { current.saturating_add(delta as u64) } else { current.saturating_sub((-delta) as u64) };
        if atomic.compare_exchange_weak(current, new_val, order, Ordering::Relaxed).is_ok() {
            break;
        }
    }
}

fn handle_worker<SL, EL, C>(
    is_running: Arc<AtomicBool>,
    worker_inbound_rx: Receiver<WorkerInbound<SL, EL>>,
    memory_usage: Arc<AtomicU64>,
    lower_time_horizon_delta: i64,
    mut apply_context: C,
) where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    EL: EventLanes<SL, C>,
    C: ApplyWrapper<SL>,
    C::Error: Into<ApplyError>,
{
    let mut history_by_id = AHashMap::<SnapshotId, SnapshotHistory<SL>>::new();
    let mut pending_inbound = VecDeque::new();

    while is_running.load(Ordering::Relaxed) {
        let inbound = match pending_inbound.pop_front() {
            Some(inbound) => Ok(inbound),
            None => worker_inbound_rx.recv(),
        };
        match inbound {
            Ok(WorkerInbound::AdvanceTime { time: new_time, reply }) => {
                for history in history_by_id.values_mut() {
                    fetch_saturating_add_signed(&memory_usage, history.advance(new_time), Ordering::Relaxed);
                }
                let _ = reply.send(());
            }
            Ok(WorkerInbound::Events { events, reply }) => {
                let (events, replies) = collect_replay_batch(events, reply, &worker_inbound_rx, &mut pending_inbound);
                let result =
                    apply_events_to_histories(&mut history_by_id, &memory_usage, lower_time_horizon_delta, &mut apply_context, events);
                for reply in replies {
                    let _ = reply.send(result.clone());
                }
            }
            Ok(WorkerInbound::SnapshotsAt { snapshot_requests, time, reply }) => {
                let mut results = Vec::with_capacity(snapshot_requests.len());
                for (position, snapshot_id) in snapshot_requests {
                    let snapshot = history_by_id.get(&snapshot_id).map(|history| history.snapshot_only_at(time));
                    results.push((position, snapshot));
                }
                let _ = reply.send(results);
            }
            Ok(WorkerInbound::Shutdown) | Err(_) => return,
        }
    }
}

fn collect_replay_batch<SL, EL>(
    mut events: Vec<WorkerEvent<SL, EL>>,
    first_reply: Sender<Result<(), ApplyError>>,
    worker_inbound_rx: &Receiver<WorkerInbound<SL, EL>>,
    pending_inbound: &mut VecDeque<WorkerInbound<SL, EL>>,
) -> (Vec<WorkerEvent<SL, EL>>, Vec<Sender<Result<(), ApplyError>>>)
where
    SL: SnapshotLanes,
{
    let mut replies = vec![first_reply];

    while let Ok(inbound) = worker_inbound_rx.try_recv() {
        match inbound {
            WorkerInbound::Events { events: next_events, reply } => {
                events.extend(next_events);
                replies.push(reply);
            }
            other => {
                pending_inbound.push_back(other);
                break;
            }
        }
    }

    (events, replies)
}

fn apply_events_to_histories<SL, EL, C>(
    history_by_id: &mut AHashMap<SnapshotId, SnapshotHistory<SL>>,
    memory_usage: &Arc<AtomicU64>,
    lower_time_horizon_delta: i64,
    apply_context: &mut C,
    events: Vec<WorkerEvent<SL, EL>>,
) -> Result<(), ApplyError>
where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    EL: EventLanes<SL, C>,
    C: ApplyWrapper<SL>,
    C::Error: Into<ApplyError>,
{
    let mut events_by_snapshot = AHashMap::<SnapshotId, (SL, Vec<EL>)>::new();
    for routed_event in events {
        match events_by_snapshot.entry(routed_event.snapshot_id) {
            Entry::Occupied(mut entry) => entry.get_mut().1.push(routed_event.event),
            Entry::Vacant(entry) => {
                entry.insert((routed_event.initial_snapshot, vec![routed_event.event]));
            }
        }
    }

    for (snapshot_id, (initial_snapshot, events)) in events_by_snapshot {
        let history = match history_by_id.entry(snapshot_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let (history, base_delta) =
                    SnapshotHistory::new_with_snapshot_id(snapshot_id, initial_snapshot, 0, lower_time_horizon_delta);
                fetch_saturating_add_signed(memory_usage, base_delta, Ordering::Relaxed);
                entry.insert(history)
            }
        };
        let bytes_delta = history.apply_event_batch(events, apply_context).map_err(Into::into)?;
        fetch_saturating_add_signed(memory_usage, bytes_delta, Ordering::Relaxed);
    }

    Ok(())
}
