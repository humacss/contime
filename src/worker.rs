use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ahash::AHashMap;
use flume::{bounded, Sender, Receiver};
use memory_stats::memory_stats;

use crate::{SnapshotHistory, SnapshotLanes, EventLanes};

pub type SnapshotId = u128;
pub type SnapshotLaneId = u32;

pub enum WorkerInbound<SL: SnapshotLanes, EL: EventLanes<SL>>
{
    SetMemoryBudget(usize),
    SnapshotAt(u128, i64),
    AdvanceTime(i64), // send new snapshot clone to subscribers on advance
    Snapshot(SL),
    Event((u128, Arc<EL>)),
    Shutdown,
}

pub enum WorkerOutbound<SL: SnapshotLanes, EL: EventLanes<SL>> {
    SnapshotAt(SL, Receiver<SL>),
    EventSkipped(Arc<EL>),
    NotifyMemoryUsage(usize),
    Error,
}

pub struct Worker<SL: SnapshotLanes, EL: EventLanes<SL>>
{
    pub worker_inbound_tx: Sender<WorkerInbound<SL, EL>>,
    pub worker_outbound_rx: Receiver<WorkerOutbound<SL, EL>>,

    threads: Vec<JoinHandle<()>>, // we don't need multiple threads anymore
    is_running: Arc<AtomicBool>,
}

impl<SL: SnapshotLanes, EL: EventLanes<SL>> Drop for Worker<SL, EL>
{
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);

        // ignore SendError since it only occurs if the worker has already shut down
        let _ = self.worker_inbound_tx.send(WorkerInbound::<SL, EL>::Shutdown);

        for thread in self.threads.drain(..) {
            thread.join().unwrap();
        }
    }
}


impl<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL> + 'static + std::marker::Send> Worker<SL, EL>
{
    pub fn new(memory_budget: usize) -> Self {
        let mut threads = Vec::with_capacity(1);
        let is_running = Arc::new(AtomicBool::new(true));

        let (worker_inbound_tx, worker_inbound_rx) = bounded::<WorkerInbound<SL, EL>>(1000);
        let (worker_outbound_tx, worker_outbound_rx) = bounded::<WorkerOutbound<SL, EL>>(1000);

        {
            let is_running = Arc::clone(&is_running);

            threads.push(thread::spawn(move || {
                handle_worker(is_running, worker_inbound_rx, worker_outbound_tx, memory_budget);
            }));
        };

        Self { 
            worker_outbound_rx,
            worker_inbound_tx,

            is_running,
            threads,
        }
    }
}

fn handle_worker<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL>> (
    is_running: Arc<AtomicBool>,
    worker_inbound_rx: Receiver<WorkerInbound<SL, EL>>,
    worker_outbound_tx: Sender<WorkerOutbound<SL, EL>>,
    mut memory_budget: usize,
){
    let mut history_by_id = AHashMap::<SnapshotId, SnapshotHistory<SL>>::new();

    let interval = Duration::from_millis(100);
    let mut last_execution = Instant::now();
    let mut current_memory_usage: usize = 0;

    while is_running.load(Ordering::Relaxed) {
        match worker_inbound_rx.recv_timeout(interval) {
            Ok(inbound) => {
                match inbound {
                    WorkerInbound::AdvanceTime(new_time) => {
                        for (_snapshot_id, history) in &history_by_id {
                            let _ = current_memory_usage.saturating_add_signed(history.advance(new_time));
                        }
                    },
                    WorkerInbound::SetMemoryBudget(new_memory_budget) => {
                        memory_budget = new_memory_budget;
                    },
                    WorkerInbound::Event((snapshot_id, event)) => {
                        let conservative_delta_bytes = event.conservative_size().saturating_add_signed(event.conservative_apply_size_delta());
                        if current_memory_usage +  conservative_delta_bytes < memory_budget {
                            let _ = current_memory_usage.saturating_add_signed(history_by_id
                                .entry(snapshot_id)
                                .or_insert(SnapshotHistory::new(SL::from_event(&event)))
                                .apply_event(event)
                            );
                        }else{
                            let _ = worker_outbound_tx.send(WorkerOutbound::<SL, EL>::EventSkipped(event));
                        }  
                    }
                    WorkerInbound::Shutdown => return,
                    WorkerInbound::Snapshot(_) => todo!(),
                    WorkerInbound::SnapshotAt(snapshot_id, time) => {
                        if let Some(history) = history_by_id.get(&snapshot_id) {
                            let (snapshot, snapshot_rx) = history.snapshot_at(time);
                            
                            worker_outbound_tx.send(WorkerOutbound::<SL, EL>::SnapshotAt(snapshot, snapshot_rx));    
                        }                        
                    },
                };
            }
            Err(_err) => {
                // does timeout also enter here?
                let _ = worker_outbound_tx.send(WorkerOutbound::<SL, EL>::Error);
                return;
            }
        }

        if last_execution.elapsed() >= interval {
            if let Some(stats) = memory_stats() {
                 current_memory_usage = stats.physical_mem;
                 let _ = worker_outbound_tx.send(WorkerOutbound::<SL, EL>::NotifyMemoryUsage(current_memory_usage));
            }
            
            last_execution = Instant::now();
        }
    }
}
