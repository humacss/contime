use std::collections::hash_map::Entry;
use std::marker::PhantomData;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ahash::AHashMap;
use crossbeam_channel::{bounded, Receiver, Sender};
use flume::Sender as FlumeSender;

use crate::handle::SnapshotWake;
use crate::handle::{QueryEventsResult, QueryResult};
use crate::{EventLanes, SnapshotHistory, SnapshotLanes};

pub type SnapshotId = u128;

pub enum WorkerInbound<SL: SnapshotLanes, EL> {
    Event { snapshot_id: u128, event: EL, initial_snapshot: SL, reply: Sender<()> },
    Snapshot { snapshot: SL, reply: Sender<()> },
    SnapshotAt { snapshot_id: u128, time: i64, reply: Sender<QueryResult<SL>> },
    SnapshotsAt { snapshot_requests: Vec<(usize, u128)>, time: i64, reply: Sender<Vec<(usize, Option<SL>)>> },
    SnapshotLanesAt { time: i64, reply: Sender<Vec<SL>> },
    EventsBetween { snapshot_id: u128, from_time: i64, to_time: i64, reply: Sender<QueryEventsResult<EL>> },
    SubscribeWakes { subscription_id: u64, tx: FlumeSender<SnapshotWake<SL>>, reply: Sender<()> },
    UnsubscribeWakes { subscription_id: u64 },
    AdvanceTime { time: i64, reply: Sender<()> },
    Shutdown,
}

pub struct Worker<SL: SnapshotLanes, EL: EventLanes<SL, C>, C = ()> {
    pub worker_inbound_tx: Sender<WorkerInbound<SL, EL>>,

    threads: Vec<JoinHandle<()>>,
    is_running: Arc<AtomicBool>,
    _context: PhantomData<C>,
}

impl<SL: SnapshotLanes, EL: EventLanes<SL, C>, C> Drop for Worker<SL, EL, C> {
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);

        // ignore SendError since it only occurs if the worker has already shut down
        let _ = self.worker_inbound_tx.send(WorkerInbound::<SL, EL>::Shutdown);

        for thread in self.threads.drain(..) {
            if let Err(e) = thread.join() {
                eprintln!("contime worker thread panicked: {:?}", e);
            }
        }
    }
}

impl<SL, EL, C> Worker<SL, EL, C>
where
    SL: SnapshotLanes<Event = EL> + 'static,
    EL: EventLanes<SL, C> + 'static + std::marker::Send,
    C: Send + 'static,
{
    pub fn new(memory_usage: Arc<AtomicU64>, apply_context: C) -> Self {
        Self::with_history_horizon(memory_usage, 0, apply_context)
    }

    pub fn with_history_horizon(memory_usage: Arc<AtomicU64>, lower_time_horizon_delta: i64, apply_context: C) -> Self {
        let mut threads = Vec::with_capacity(1);
        let is_running = Arc::new(AtomicBool::new(true));

        let (worker_inbound_tx, worker_inbound_rx) = bounded::<WorkerInbound<SL, EL>>(1000);

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

fn handle_worker<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL, C>, C>(
    is_running: Arc<AtomicBool>,
    worker_inbound_rx: Receiver<WorkerInbound<SL, EL>>,
    memory_usage: Arc<AtomicU64>,
    lower_time_horizon_delta: i64,
    mut apply_context: C,
) {
    let mut history_by_id = AHashMap::<SnapshotId, SnapshotHistory<SL>>::new();
    let mut wake_subscribers = AHashMap::<u64, FlumeSender<SnapshotWake<SL>>>::new();

    while is_running.load(Ordering::Relaxed) {
        match worker_inbound_rx.recv() {
            Ok(inbound) => {
                match inbound {
                    WorkerInbound::AdvanceTime { time: new_time, reply } => {
                        for (_snapshot_id, history) in &mut history_by_id {
                            fetch_saturating_add_signed(&memory_usage, history.advance(new_time), Ordering::Relaxed);
                        }
                        // Ignore SendError: caller may have dropped the handle, which is fine
                        let _ = reply.send(());
                    }
                    WorkerInbound::Event { snapshot_id, event, initial_snapshot, reply } => {
                        let history = match history_by_id.entry(snapshot_id) {
                            Entry::Occupied(entry) => entry.into_mut(),
                            Entry::Vacant(entry) => {
                                let (history, base_delta) = SnapshotHistory::new(initial_snapshot, 0, lower_time_horizon_delta);
                                fetch_saturating_add_signed(&memory_usage, base_delta, Ordering::Relaxed);
                                entry.insert(history)
                            }
                        };
                        let outcome = history.apply_event_with_context(event, &mut apply_context);
                        fetch_saturating_add_signed(&memory_usage, outcome.bytes_delta, Ordering::Relaxed);
                        if let Some(wake) = outcome.wake {
                            notify_wakes(
                                &mut wake_subscribers,
                                SnapshotWake {
                                    lane: history.snapshot_only_at(wake.to_time.saturating_add(1)),
                                    snapshot_id,
                                    from_time: wake.from_time,
                                    to_time: wake.to_time,
                                    cause: wake.cause,
                                },
                            );
                        }
                        let _ = reply.send(());
                    }
                    WorkerInbound::Snapshot { snapshot, reply } => {
                        let snapshot_id = snapshot.id();

                        let outcome = match history_by_id.entry(snapshot_id) {
                            Entry::Occupied(mut entry) => entry.get_mut().apply_snapshot(snapshot),
                            Entry::Vacant(entry) => {
                                let snapshot_time = snapshot.time();
                                let (history, base_delta) = SnapshotHistory::new(snapshot, 0, lower_time_horizon_delta);
                                entry.insert(history);
                                crate::history::ApplyOutcome {
                                    bytes_delta: base_delta,
                                    wake: Some(crate::history::SnapshotWakeWindow {
                                        from_time: snapshot_time,
                                        to_time: snapshot_time,
                                        cause: crate::history::SnapshotWakeCause::Changed,
                                    }),
                                }
                            }
                        };

                        fetch_saturating_add_signed(&memory_usage, outcome.bytes_delta, Ordering::Relaxed);
                        if let Some(wake) = outcome.wake {
                            if let Some(history) = history_by_id.get(&snapshot_id) {
                                notify_wakes(
                                    &mut wake_subscribers,
                                    SnapshotWake {
                                        lane: history.snapshot_only_at(wake.to_time.saturating_add(1)),
                                        snapshot_id,
                                        from_time: wake.from_time,
                                        to_time: wake.to_time,
                                        cause: wake.cause,
                                    },
                                );
                            }
                        }
                        let _ = reply.send(());
                    }

                    WorkerInbound::SnapshotAt { snapshot_id, time, reply } => {
                        if let Some(history) = history_by_id.get_mut(&snapshot_id) {
                            let (snapshot, reconciliation_rx) = history.snapshot_at(time);
                            let _ = reply.send(QueryResult::Found(snapshot, reconciliation_rx));
                        } else {
                            let _ = reply.send(QueryResult::NotFound);
                        }
                    }
                    WorkerInbound::SnapshotsAt { snapshot_requests, time, reply } => {
                        let mut results = Vec::with_capacity(snapshot_requests.len());
                        for (position, snapshot_id) in snapshot_requests {
                            let snapshot = history_by_id.get(&snapshot_id).map(|history| history.snapshot_only_at(time));
                            results.push((position, snapshot));
                        }
                        let _ = reply.send(results);
                    }
                    WorkerInbound::SnapshotLanesAt { time, reply } => {
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
                    WorkerInbound::EventsBetween { snapshot_id, from_time, to_time, reply } => {
                        let result = history_by_id
                            .get(&snapshot_id)
                            .map(|history| QueryEventsResult::Found(history.events_between(from_time, to_time)))
                            .unwrap_or(QueryEventsResult::NotFound);
                        let _ = reply.send(result);
                    }
                    WorkerInbound::SubscribeWakes { subscription_id, tx, reply } => {
                        wake_subscribers.insert(subscription_id, tx);
                        let _ = reply.send(());
                    }
                    WorkerInbound::UnsubscribeWakes { subscription_id } => {
                        wake_subscribers.remove(&subscription_id);
                    }
                    WorkerInbound::Shutdown => return,
                };
            }
            Err(_) => return,
        }
    }
}

fn notify_wakes<SL: SnapshotLanes>(subscribers: &mut AHashMap<u64, FlumeSender<SnapshotWake<SL>>>, wake: SnapshotWake<SL>) {
    subscribers.retain(|_, tx| match tx.send(wake.clone()) {
        Ok(()) => true,
        Err(flume::SendError(_)) => false,
    });
}
