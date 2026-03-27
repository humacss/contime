use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::collections::hash_map::Entry;

use ahash::AHashMap;
use flume::{bounded, Sender, Receiver};

use crate::{SnapshotHistory, SnapshotLanes, EventLanes};
use crate::handle::QueryResult;

pub type SnapshotId = u128;

pub enum WorkerInbound<SL: SnapshotLanes, EL: EventLanes<SL>>
{
    Event { snapshot_id: u128, event: EL, initial_snapshot: SL, reply: Sender<()> },
    Snapshot { snapshot: SL, reply: Sender<()> },
    SnapshotAt { snapshot_id: u128, time: i64, reply: Sender<QueryResult<SL>> },
    AdvanceTime { time: i64, reply: Sender<()> },
    Shutdown,
}

pub struct Worker<SL: SnapshotLanes, EL: EventLanes<SL>>
{
    pub worker_inbound_tx: Sender<WorkerInbound<SL, EL>>,

    threads: Vec<JoinHandle<()>>,
    is_running: Arc<AtomicBool>,
}

impl<SL: SnapshotLanes, EL: EventLanes<SL>> Drop for Worker<SL, EL>
{
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


impl<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL> + 'static + std::marker::Send> Worker<SL, EL>
{
    pub fn new(memory_usage: Arc<AtomicU64>) -> Self {
        Self::with_history_horizon(memory_usage, 0)
    }

    pub fn with_history_horizon(
        memory_usage: Arc<AtomicU64>,
        lower_time_horizon_delta: i64,
    ) -> Self {
        let mut threads = Vec::with_capacity(1);
        let is_running = Arc::new(AtomicBool::new(true));

        let (worker_inbound_tx, worker_inbound_rx) = bounded::<WorkerInbound<SL, EL>>(1000);

        {
            let is_running = Arc::clone(&is_running);
            let memory_usage = Arc::clone(&memory_usage);

            threads.push(thread::spawn(move || {
                handle_worker(
                    is_running,
                    worker_inbound_rx,
                    memory_usage,
                    lower_time_horizon_delta,
                );
            }));
        };

        Self {
            worker_inbound_tx,

            is_running,
            threads,
        }
    }
}

fn fetch_saturating_add_signed(atomic: &Arc<AtomicU64>, delta: i64, order: Ordering) {
    loop {
        let current = atomic.load(order);
        let new_val = if delta >= 0 {
            current.saturating_add(delta as u64)
        } else {
            current.saturating_sub((-delta) as u64)
        };
        if atomic.compare_exchange_weak(current, new_val, order, Ordering::Relaxed).is_ok() {
            break;
        }
    }
}

fn handle_worker<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL>> (
    is_running: Arc<AtomicBool>,
    worker_inbound_rx: Receiver<WorkerInbound<SL, EL>>,
    memory_usage: Arc<AtomicU64>,
    lower_time_horizon_delta: i64,
){
    let mut history_by_id = AHashMap::<SnapshotId, SnapshotHistory<SL>>::new();

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
                    },
                    WorkerInbound::Event { snapshot_id, event, initial_snapshot, reply } => {
                        let history = match history_by_id.entry(snapshot_id) {
                            Entry::Occupied(entry) => entry.into_mut(),
                            Entry::Vacant(entry) => {
                                let (history, base_delta) = SnapshotHistory::new(
                                    initial_snapshot,
                                    0,
                                    lower_time_horizon_delta,
                                );
                                fetch_saturating_add_signed(&memory_usage, base_delta, Ordering::Relaxed);
                                entry.insert(history)
                            }
                        };
                        let delta = history.apply_event(event);
                        fetch_saturating_add_signed(&memory_usage, delta, Ordering::Relaxed);
                        let _ = reply.send(());
                    }
                    WorkerInbound::Snapshot { snapshot, reply } => {
                        let snapshot_id = snapshot.id();

                        let apply_delta = match history_by_id.entry(snapshot_id) {
                            Entry::Occupied(mut entry) => {
                                entry.get_mut().apply_snapshot(snapshot)
                            },
                            Entry::Vacant(entry) => {
                                let (history, base_delta) = SnapshotHistory::new(
                                    snapshot,
                                    0,
                                    lower_time_horizon_delta,
                                );
                                entry.insert(history);
                                base_delta
                            }
                        };

                        fetch_saturating_add_signed(&memory_usage, apply_delta, Ordering::Relaxed);
                        let _ = reply.send(());
                    },

                    WorkerInbound::SnapshotAt { snapshot_id, time, reply } => {
                        if let Some(history) = history_by_id.get(&snapshot_id) {
                            let (snapshot, reconciliation_rx) = history.snapshot_at(time);
                            let _ = reply.send(QueryResult::Found(snapshot, reconciliation_rx));
                        } else {
                            let _ = reply.send(QueryResult::NotFound);
                        }
                    },
                    WorkerInbound::Shutdown => return,
                };
            }
            Err(_) => return,
        }
    }
}
