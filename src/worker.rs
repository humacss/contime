use std::collections::hash_map::Entry;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::ops::Bound;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ahash::{AHashMap, RandomState};
use crossbeam_channel::{Receiver, Sender};

use crate::{ApplyEvents, ApplyWrapper, ContimeKey, ContimeTime, Event, EventJournalEntry, EventLanes, SnapshotHistory, SnapshotLanes};

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
    EventsInRange { start: Bound<SL::Time>, end: Bound<SL::Time>, reply: Sender<Vec<EventJournalEntry<EL>>> },
    SnapshotsAt { snapshot_requests: Vec<(usize, u128)>, time: SL::Time, reply: Sender<Result<Vec<(usize, Option<SL>)>, ApplyError>> },
    AdvanceTime { time: SL::Time, reply: Sender<Result<(), ApplyError>> },
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
        lower_time_horizon_delta: SL::Time,
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
    lower_time_horizon_delta: SL::Time,
    mut apply_context: C,
) where
    SL: SnapshotLanes<Event = EL> + ApplyEvents + 'static,
    EL: EventLanes<SL, C>,
    C: ApplyWrapper<SL>,
    C::Error: Into<ApplyError>,
{
    let mut history_by_id = AHashMap::<SnapshotId, SnapshotHistory<SL>>::new();
    let mut event_log = Vec::<EventJournalEntry<EL>>::new();
    let mut pending_inbound = VecDeque::new();

    while is_running.load(Ordering::Relaxed) {
        let inbound = match pending_inbound.pop_front() {
            Some(inbound) => Ok(inbound),
            None => worker_inbound_rx.recv(),
        };
        match inbound {
            Ok(WorkerInbound::AdvanceTime { time: new_time, reply }) => {
                let mut result = Ok(());
                for history in history_by_id.values_mut() {
                    match history.advance_with_context(new_time.clone(), &mut apply_context) {
                        Ok(bytes_delta) => fetch_saturating_add_signed(&memory_usage, bytes_delta, Ordering::Relaxed),
                        Err(error) => {
                            result = Err(error.into());
                            break;
                        }
                    }
                }
                if result.is_ok() {
                    let drop_time = new_time.saturating_sub(lower_time_horizon_delta.clone());
                    let bytes_removed = prune_event_log(&mut event_log, drop_time);
                    fetch_saturating_add_signed(&memory_usage, -bytes_removed, Ordering::Relaxed);
                }
                let _ = reply.send(result);
            }
            Ok(WorkerInbound::Events { events, reply }) => {
                let (events, replies) = collect_replay_batch(events, reply, &worker_inbound_rx, &mut pending_inbound);
                let bytes_added = record_worker_events(&mut event_log, &events);
                fetch_saturating_add_signed(&memory_usage, bytes_added, Ordering::Relaxed);
                let result = apply_events_to_histories(
                    &mut history_by_id,
                    &memory_usage,
                    lower_time_horizon_delta.clone(),
                    &mut apply_context,
                    events,
                );
                for reply in replies {
                    let _ = reply.send(result.clone());
                }
            }
            Ok(WorkerInbound::EventsInRange { start, end, reply }) => {
                let events = event_log.iter().filter(|entry| time_is_in_range(entry.event.time(), &start, &end)).cloned().collect();
                let _ = reply.send(events);
            }
            Ok(WorkerInbound::SnapshotsAt { snapshot_requests, time, reply }) => {
                let mut results = Vec::with_capacity(snapshot_requests.len());
                let mut error = None;
                for (position, snapshot_id) in snapshot_requests {
                    let snapshot = match history_by_id.get(&snapshot_id) {
                        Some(history) => match history.snapshot_only_at_with_context(time.clone(), &mut apply_context) {
                            Ok(snapshot) => Some(snapshot),
                            Err(err) => {
                                error = Some(err.into());
                                break;
                            }
                        },
                        None => None,
                    };
                    results.push((position, snapshot));
                }
                let _ = reply.send(match error {
                    Some(error) => Err(error),
                    None => Ok(results),
                });
            }
            Ok(WorkerInbound::Shutdown) | Err(_) => return,
        }
    }
}

fn record_worker_events<SL, EL>(event_log: &mut Vec<EventJournalEntry<EL>>, events: &[WorkerEvent<SL, EL>]) -> i64
where
    SL: SnapshotLanes,
    EL: Event + Clone,
{
    let mut bytes_added = 0i64;
    for routed_event in events {
        let key = ContimeKey::from_event(&routed_event.event);
        match event_log.binary_search_by_key(&key, |entry| ContimeKey::from_event(&entry.event)) {
            Ok(index) => {
                let snapshot_ids = &mut event_log[index].routed_snapshot_ids;
                if let Err(route_index) = snapshot_ids.binary_search(&routed_event.snapshot_id) {
                    snapshot_ids.insert(route_index, routed_event.snapshot_id);
                    bytes_added = bytes_added.saturating_add(size_of::<u128>() as i64);
                }
            }
            Err(index) => {
                let entry = EventJournalEntry { event: routed_event.event.clone(), routed_snapshot_ids: vec![routed_event.snapshot_id] };
                bytes_added = bytes_added.saturating_add(entry.conservative_size() as i64);
                event_log.insert(index, entry);
            }
        }
    }
    bytes_added
}

fn prune_event_log<EL>(event_log: &mut Vec<EventJournalEntry<EL>>, time: EL::Time) -> i64
where
    EL: Event,
{
    let drop_key = ContimeKey { time, id: u128::MIN };
    let first_kept = event_log.partition_point(|entry| ContimeKey::from_event(&entry.event) < drop_key);
    let bytes_removed = event_log[..first_kept].iter().fold(0i64, |size, entry| size.saturating_add(entry.conservative_size() as i64));
    event_log.drain(..first_kept);
    bytes_removed
}

fn time_is_in_range<T: crate::ContimeTime>(time: T, start: &Bound<T>, end: &Bound<T>) -> bool {
    let after_start = match start {
        Bound::Included(start) => time >= *start,
        Bound::Excluded(start) => time > *start,
        Bound::Unbounded => true,
    };
    let before_end = match end {
        Bound::Included(end) => time <= *end,
        Bound::Excluded(end) => time < *end,
        Bound::Unbounded => true,
    };
    after_start && before_end
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
    lower_time_horizon_delta: SL::Time,
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
                let (history, base_delta) = SnapshotHistory::new_with_snapshot_id(
                    snapshot_id,
                    initial_snapshot,
                    SL::Time::default(),
                    lower_time_horizon_delta.clone(),
                );
                fetch_saturating_add_signed(memory_usage, base_delta, Ordering::Relaxed);
                entry.insert(history)
            }
        };
        let bytes_delta = history.apply_event_batch(events, apply_context).map_err(Into::into)?;
        fetch_saturating_add_signed(memory_usage, bytes_delta, Ordering::Relaxed);
    }

    Ok(())
}
