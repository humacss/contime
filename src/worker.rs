use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, VecDeque};
use std::marker::PhantomData;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ahash::AHashMap;
use crossbeam_channel::{Receiver, Sender};

use crate::handle::{QueryEventsResult, QueryResult};
use crate::{AfterApplyEvents, ApplyEvents, EventLanes, ScheduleKey, SnapshotHistory, SnapshotLanes};

pub type SnapshotId = u128;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ScheduledEventKey {
    time: i64,
    event_id: u128,
    schedule_key: ScheduleKey,
    snapshot_id: u128,
}

struct ScheduledEvent<SL, EL> {
    snapshot_id: u128,
    schedule_key: ScheduleKey,
    event: EL,
    initial_snapshot: SL,
}

impl<SL: SnapshotLanes, EL: crate::Event> ScheduledEvent<SL, EL> {
    fn key(&self) -> ScheduledEventKey {
        ScheduledEventKey {
            time: self.event.time(),
            event_id: self.event.id(),
            schedule_key: self.schedule_key,
            snapshot_id: self.snapshot_id,
        }
    }

    fn conservative_size(&self) -> u64 {
        self.event.conservative_size().saturating_add(self.initial_snapshot.conservative_size())
    }
}

pub enum WorkerInbound<SL: SnapshotLanes, EL> {
    Event { snapshot_id: u128, event: EL, initial_snapshot: SL, reply: Sender<()> },
    ScheduleEvent { snapshot_id: u128, schedule_key: ScheduleKey, event: EL, initial_snapshot: SL, reply: Sender<()> },
    CancelScheduledEvent { event_id: u128, event_time: i64, reply: Sender<()> },
    Snapshot { snapshot: SL, reply: Sender<()> },
    SnapshotAt { snapshot_id: u128, time: i64, reply: Sender<QueryResult<SL>> },
    SnapshotsAt { snapshot_requests: Vec<(usize, u128)>, time: i64, reply: Sender<Vec<(usize, Option<SL>)>> },
    SnapshotLanesAt { time: i64, reply: Sender<Vec<SL>> },
    EventsBetween { snapshot_id: u128, from_time: i64, to_time: i64, reply: Sender<QueryEventsResult<EL>> },
    AdvanceTime { time: i64, reply: Sender<()> },
    Shutdown,
}

pub struct Worker<SL: SnapshotLanes + ApplyEvents + AfterApplyEvents<C>, EL: EventLanes<SL, C>, C = ()> {
    pub worker_inbound_tx: Sender<WorkerInbound<SL, EL>>,

    threads: Vec<JoinHandle<()>>,
    is_running: Arc<AtomicBool>,
    _context: PhantomData<C>,
}

impl<SL: SnapshotLanes + ApplyEvents + AfterApplyEvents<C>, EL: EventLanes<SL, C>, C> Drop for Worker<SL, EL, C> {
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

fn handle_worker<SL: SnapshotLanes<Event = EL> + ApplyEvents + AfterApplyEvents<C> + 'static, EL: EventLanes<SL, C>, C>(
    is_running: Arc<AtomicBool>,
    worker_inbound_rx: Receiver<WorkerInbound<SL, EL>>,
    memory_usage: Arc<AtomicU64>,
    lower_time_horizon_delta: i64,
    mut apply_context: C,
) {
    let mut history_by_id = AHashMap::<SnapshotId, SnapshotHistory<SL>>::new();
    let mut scheduled_events = BTreeMap::<ScheduledEventKey, ScheduledEvent<SL, EL>>::new();
    let mut scheduled_event_keys_by_identity = AHashMap::<(ScheduleKey, SnapshotId), ScheduledEventKey>::new();
    let mut ready_events = VecDeque::<ScheduledEvent<SL, EL>>::new();
    let mut current_time = 0_i64;

    while is_running.load(Ordering::Relaxed) {
        if let Some(event) = ready_events.pop_front() {
            apply_event_to_history(
                &mut history_by_id,
                &memory_usage,
                lower_time_horizon_delta,
                &mut apply_context,
                event.snapshot_id,
                event.event,
                event.initial_snapshot,
            );
            continue;
        }

        match worker_inbound_rx.recv() {
            Ok(WorkerInbound::AdvanceTime { time: new_time, reply }) => {
                current_time = current_time.saturating_add(new_time);
                for history in history_by_id.values_mut() {
                    fetch_saturating_add_signed(&memory_usage, history.advance(new_time), Ordering::Relaxed);
                }
                let due_to = ScheduledEventKey { time: current_time, event_id: u128::MAX, schedule_key: u128::MAX, snapshot_id: u128::MAX };
                let due_keys = scheduled_events.range(..=due_to).map(|(key, _)| key.clone()).collect::<Vec<_>>();
                for key in due_keys {
                    if let Some(event) = scheduled_events.remove(&key) {
                        scheduled_event_keys_by_identity.remove(&(event.schedule_key, event.snapshot_id));
                        fetch_saturating_add_signed(&memory_usage, -(event.conservative_size() as i64), Ordering::Relaxed);
                        ready_events.push_back(event);
                    }
                }
                drain_ready_events(&mut ready_events, &mut history_by_id, &memory_usage, lower_time_horizon_delta, &mut apply_context);
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
                drain_ready_events(&mut ready_events, &mut history_by_id, &memory_usage, lower_time_horizon_delta, &mut apply_context);
                let _ = reply.send(());
            }
            Ok(WorkerInbound::ScheduleEvent { snapshot_id, schedule_key, event, initial_snapshot, reply }) => {
                let scheduled = ScheduledEvent { snapshot_id, schedule_key, event, initial_snapshot };
                if let Some(previous_key) = scheduled_event_keys_by_identity.remove(&(schedule_key, snapshot_id)) {
                    if let Some(previous) = scheduled_events.remove(&previous_key) {
                        fetch_saturating_add_signed(&memory_usage, -(previous.conservative_size() as i64), Ordering::Relaxed);
                    }
                }
                let key = scheduled.key();
                if key.time <= current_time {
                    ready_events.push_back(scheduled);
                } else {
                    fetch_saturating_add_signed(&memory_usage, scheduled.conservative_size() as i64, Ordering::Relaxed);
                    scheduled_event_keys_by_identity.insert((schedule_key, snapshot_id), key.clone());
                    scheduled_events.insert(key, scheduled);
                }
                let _ = reply.send(());
            }
            Ok(WorkerInbound::CancelScheduledEvent { event_id, event_time, reply }) => {
                let from = ScheduledEventKey { time: event_time, event_id, schedule_key: u128::MIN, snapshot_id: u128::MIN };
                let to = ScheduledEventKey { time: event_time, event_id, schedule_key: u128::MAX, snapshot_id: u128::MAX };
                let keys = scheduled_events.range(from..=to).map(|(key, _)| key.clone()).collect::<Vec<_>>();
                for key in keys {
                    if let Some(event) = scheduled_events.remove(&key) {
                        scheduled_event_keys_by_identity.remove(&(event.schedule_key, event.snapshot_id));
                        fetch_saturating_add_signed(&memory_usage, -(event.conservative_size() as i64), Ordering::Relaxed);
                    }
                }
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
                drain_ready_events(&mut ready_events, &mut history_by_id, &memory_usage, lower_time_horizon_delta, &mut apply_context);
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
            Ok(WorkerInbound::EventsBetween { snapshot_id, from_time, to_time, reply }) => {
                let result = history_by_id
                    .get(&snapshot_id)
                    .map(|history| QueryEventsResult::Found(history.events_between(from_time, to_time)))
                    .unwrap_or(QueryEventsResult::NotFound);
                let _ = reply.send(result);
            }
            Ok(WorkerInbound::Shutdown) | Err(_) => return,
        }
    }
}

fn apply_event_to_history<SL: SnapshotLanes<Event = EL> + ApplyEvents + AfterApplyEvents<C> + 'static, EL: EventLanes<SL, C>, C>(
    history_by_id: &mut AHashMap<SnapshotId, SnapshotHistory<SL>>,
    memory_usage: &Arc<AtomicU64>,
    lower_time_horizon_delta: i64,
    apply_context: &mut C,
    snapshot_id: u128,
    event: EL,
    initial_snapshot: SL,
) {
    let history = match history_by_id.entry(snapshot_id) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            let (history, base_delta) = SnapshotHistory::new(initial_snapshot, 0, lower_time_horizon_delta);
            fetch_saturating_add_signed(memory_usage, base_delta, Ordering::Relaxed);
            entry.insert(history)
        }
    };
    let outcome = history.apply_event_with_context(event, apply_context);
    fetch_saturating_add_signed(memory_usage, outcome.bytes_delta, Ordering::Relaxed);
}

fn drain_ready_events<SL: SnapshotLanes<Event = EL> + ApplyEvents + AfterApplyEvents<C> + 'static, EL: EventLanes<SL, C>, C>(
    ready_events: &mut VecDeque<ScheduledEvent<SL, EL>>,
    history_by_id: &mut AHashMap<SnapshotId, SnapshotHistory<SL>>,
    memory_usage: &Arc<AtomicU64>,
    lower_time_horizon_delta: i64,
    apply_context: &mut C,
) {
    while let Some(event) = ready_events.pop_front() {
        apply_event_to_history(
            history_by_id,
            memory_usage,
            lower_time_horizon_delta,
            apply_context,
            event.snapshot_id,
            event.event,
            event.initial_snapshot,
        );
    }
}
