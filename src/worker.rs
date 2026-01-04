use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use flume::{bounded, Receiver, Sender};

use crate::{ApplyEvent, Snapshot, SnapshotHistory};

pub type SnapshotId = u128;

pub enum WorkerInbound<S>
where
    S: Snapshot,
{
    Event(S::Event),
    Shutdown,
}

pub enum WorkerOutbound<S>
where
    S: Snapshot,
{
    Snapshot(S),
}

pub struct Worker<S>
where
    S: Snapshot,
{
    threads: Vec<JoinHandle<()>>, // we don't need multiple threads anymore
    is_running: Arc<AtomicBool>,

    worker_inbound_tx: Sender<WorkerInbound<S>>,
    worker_outbound_rx: Receiver<WorkerOutbound<S>>,
}

impl<S> Drop for Worker<S>
where
    S: Snapshot,
{
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);

        // ignore SendError since it only occurs if the worker has already shut down
        let _ = self.worker_inbound_tx.send(WorkerInbound::Shutdown);

        for thread in self.threads.drain(..) {
            thread.join().unwrap();
        }
    }
}

impl<S> Worker<S>
where
    S: Snapshot + 'static,
{
    pub fn new() -> Self {
        let mut threads = Vec::with_capacity(1);
        let is_running = Arc::new(AtomicBool::new(true));

        let (worker_inbound_tx, worker_inbound_rx) = bounded::<WorkerInbound<S>>(1000);
        let (worker_outbound_tx, worker_outbound_rx) = bounded::<WorkerOutbound<S>>(1000);

        {
            let is_running = Arc::clone(&is_running);

            threads.push(thread::spawn(move || {
                handle_worker(is_running, worker_inbound_rx, worker_outbound_tx);
            }));
        };

        Self { is_running, threads, worker_outbound_rx, worker_inbound_tx }
    }

    pub fn get_tx(&self) -> flume::Sender<WorkerInbound<S>> {
        self.worker_inbound_tx.clone()
    }

    pub fn get_rx(&self) -> flume::Receiver<WorkerOutbound<S>> {
        self.worker_outbound_rx.clone()
    }
}

fn handle_worker<S: Snapshot + 'static>(
    is_running: Arc<AtomicBool>,
    inbound_rx: Receiver<WorkerInbound<S>>,
    _outbound_tx: Sender<WorkerOutbound<S>>,
) where
    <S as Snapshot>::Event: 'static,
{
    let mut history_by_id = HashMap::<SnapshotId, SnapshotHistory<S>>::new();

    while is_running.load(Ordering::Relaxed) {
        match inbound_rx.recv() {
            Ok(inbound) => {
                match inbound {
                    WorkerInbound::Event(event) => {
                        history_by_id
                            .entry(event.snapshot_id())
                            .or_insert(SnapshotHistory::<S>::new(event.snapshot_id()))
                            .apply_event(event);
                    }
                    WorkerInbound::Shutdown => return,
                };
            }
            Err(err) => {
                panic!("{:?}", err);
            }
        }
    }
}
