use std::collections::hash_map::Entry;
use std::marker::PhantomData;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ahash::{AHashMap, RandomState};
use crossbeam_channel::{Receiver, Sender};

use crate::handle::QueryResult;
use crate::{AfterApplyEvents, ApplyEvents, EventLanes, SnapshotHistory, SnapshotLanes};

pub type SnapshotId = u128;

pub enum WorkerInbound<SL: SnapshotLanes, EL> {
    Event { snapshot_id: u128, event: EL, initial_snapshot: SL, reply: Sender<()> },
    ReplaceContextEvents { source_key: u128, from_time: i64, events: Vec<EL>, reply: Sender<()> },
    Snapshot { snapshot: SL, reply: Sender<()> },
    SnapshotAt { snapshot_id: u128, time: i64, reply: Sender<QueryResult<SL>> },
    SnapshotsAt { snapshot_requests: Vec<(usize, u128)>, time: i64, reply: Sender<Vec<(usize, Option<SL>)>> },
    SnapshotLanesAt { time: i64, reply: Sender<Vec<SL>> },
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
    SL: SnapshotLanes<Event = EL> + ApplyEvents + AfterApplyEvents<C> + 'static,
    EL: EventLanes<SL, C> + 'static + Send,
    C: Send + 'static,
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
    SL: SnapshotLanes<Event = EL> + ApplyEvents + AfterApplyEvents<C> + 'static,
    EL: EventLanes<SL, C>,
{
    let mut history_by_id = AHashMap::<SnapshotId, SnapshotHistory<SL>>::new();

    while is_running.load(Ordering::Relaxed) {
        match worker_inbound_rx.recv() {
            Ok(WorkerInbound::AdvanceTime { time: new_time, reply }) => {
                for history in history_by_id.values_mut() {
                    fetch_saturating_add_signed(&memory_usage, history.advance(new_time), Ordering::Relaxed);
                }
                let _ = reply.send(());
            }
            Ok(WorkerInbound::Event { snapshot_id, event, initial_snapshot, reply }) => {
                apply_event_to_history(
                    &mut history_by_id,
                    &memory_usage,
                    lower_time_horizon_delta,
                    &mut apply_context,
                    snapshot_id,
                    event,
                    initial_snapshot,
                );
                let _ = reply.send(());
            }
            Ok(WorkerInbound::ReplaceContextEvents { source_key, from_time, events, reply }) => {
                replace_context_events_in_worker(
                    &mut history_by_id,
                    &memory_usage,
                    lower_time_horizon_delta,
                    &mut apply_context,
                    source_key,
                    from_time,
                    events,
                );
                let _ = reply.send(());
            }
            Ok(WorkerInbound::Snapshot { snapshot, reply }) => {
                let snapshot_id = snapshot.id();
                let outcome = match history_by_id.entry(snapshot_id) {
                    Entry::Occupied(mut entry) => entry.get_mut().apply_snapshot(snapshot),
                    Entry::Vacant(entry) => {
                        let (history, base_delta) = SnapshotHistory::new(snapshot, 0, lower_time_horizon_delta);
                        entry.insert(history);
                        crate::history::ApplyOutcome { bytes_delta: base_delta }
                    }
                };

                fetch_saturating_add_signed(&memory_usage, outcome.bytes_delta, Ordering::Relaxed);
                let _ = reply.send(());
            }
            Ok(WorkerInbound::SnapshotAt { snapshot_id, time, reply }) => {
                if let Some(history) = history_by_id.get_mut(&snapshot_id) {
                    let (snapshot, reconciliation_rx) = history.snapshot_at(time);
                    let _ = reply.send(QueryResult::Found(snapshot, reconciliation_rx));
                } else {
                    let _ = reply.send(QueryResult::NotFound);
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
            Ok(WorkerInbound::SnapshotLanesAt { time, reply }) => {
                let mut snapshot_ids = history_by_id.keys().copied().collect::<Vec<_>>();
                snapshot_ids.sort_unstable();

                let mut lanes = Vec::with_capacity(snapshot_ids.len());
                for snapshot_id in snapshot_ids {
                    if let Some(history) = history_by_id.get(&snapshot_id) {
                        lanes.push(history.snapshot_only_at(time));
                    }
                }
                let _ = reply.send(lanes);
            }
            Ok(WorkerInbound::Shutdown) | Err(_) => return,
        }
    }
}

fn replace_context_events_in_worker<SL, EL, C>(
    history_by_id: &mut AHashMap<SnapshotId, SnapshotHistory<SL>>,
    memory_usage: &Arc<AtomicU64>,
    lower_time_horizon_delta: i64,
    apply_context: &mut C,
    source_key: u128,
    from_time: i64,
    events: Vec<EL>,
) where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + AfterApplyEvents<C> + 'static,
    EL: EventLanes<SL, C>,
{
    let mut events_by_snapshot = AHashMap::<SnapshotId, (SL, Vec<EL>)>::new();
    for event in events {
        for routed in event.routed_snapshots() {
            events_by_snapshot.entry(routed.snapshot_id).or_insert_with(|| (routed.initial_snapshot, Vec::new())).1.push(event.clone());
        }
    }

    let existing_snapshot_ids = history_by_id.keys().copied().collect::<Vec<_>>();
    for snapshot_id in existing_snapshot_ids {
        let events = events_by_snapshot.remove(&snapshot_id).map(|(_, events)| events).unwrap_or_default();
        let Some(history) = history_by_id.get_mut(&snapshot_id) else {
            continue;
        };
        let outcome = history.replace_context_events(source_key, from_time, events, apply_context);
        fetch_saturating_add_signed(memory_usage, outcome.bytes_delta, Ordering::Relaxed);
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
        let outcome = history.replace_context_events(source_key, from_time, events, apply_context);
        fetch_saturating_add_signed(memory_usage, outcome.bytes_delta, Ordering::Relaxed);
    }
}

fn apply_event_to_history<SL, EL, C>(
    history_by_id: &mut AHashMap<SnapshotId, SnapshotHistory<SL>>,
    memory_usage: &Arc<AtomicU64>,
    lower_time_horizon_delta: i64,
    apply_context: &mut C,
    snapshot_id: u128,
    event: EL,
    initial_snapshot: SL,
) where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + AfterApplyEvents<C> + 'static,
    EL: EventLanes<SL, C>,
{
    let history = match history_by_id.entry(snapshot_id) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            let (history, base_delta) = SnapshotHistory::new_with_snapshot_id(snapshot_id, initial_snapshot, 0, lower_time_horizon_delta);
            fetch_saturating_add_signed(memory_usage, base_delta, Ordering::Relaxed);
            entry.insert(history)
        }
    };
    let outcome = history.apply_event_with_context(event, apply_context);
    fetch_saturating_add_signed(memory_usage, outcome.bytes_delta, Ordering::Relaxed);
}
