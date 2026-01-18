use std::sync::Arc;
use std::time::Duration;

use ahash::RandomState;
use flume::Receiver;

use crate::{Worker, SnapshotLanes, EventLanes, WorkerInbound, ContimeError, WorkerOutbound};

#[derive(Debug)]
pub enum RouterError {
    Timeout
}

impl From<RouterError> for ContimeError {
    fn from(err: RouterError) -> Self {
        ContimeError::RouterError(err)
    }
}

pub enum RouterInbound<SL: SnapshotLanes, EL: EventLanes<SL>>
{
    Snapshot(SL),
    Event(EL),
}

pub enum RouterOutbound<SL: SnapshotLanes, EL: EventLanes<SL>> {
    Reconciliate(SL, i64, i64),
    RejectedEvent(EL, u128),
}

pub struct Router<SL: SnapshotLanes<Event = EL>, EL: EventLanes<SL>>
{
    hasher: RandomState,
    memory_budget_bytes: usize,
    workers: Vec<Worker<SL, EL>>,
}

impl<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL> + 'static> Router<SL, EL> {
    pub fn new(
        worker_count: usize, 
        memory_budget_bytes: usize
    ) -> Self {
        let hasher = RandomState::new();
        let worker_memory_budget = memory_budget_bytes / worker_count;

        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            workers.push(Worker::new(worker_memory_budget));
        }

        Self { hasher, memory_budget_bytes, workers }
    }

    pub fn send(&self, event_lane: EL) -> Result<(), RouterError> {
        let event_lane = Arc::new(event_lane);

        for snapshot in event_lane.snapshots() {
            let index = self.worker_index(snapshot.id());

            let _ = self.workers[index].worker_inbound_tx.send(WorkerInbound::Event((snapshot.id(), Arc::clone(&event_lane))));    
        }

        Ok(())
    }

    pub fn at(&self, time: i64, snapshot_id: u128) -> Result<(SL, Receiver<SL>), RouterError> {
        let index = self.worker_index(snapshot_id);

        let _ = self.workers[index].worker_inbound_tx.send(WorkerInbound::SnapshotAt(snapshot_id, time));

        // add a timeout
        loop {
            match self.workers[index].worker_outbound_rx.recv_timeout(Duration::from_millis(1)) {
                Ok(message) => {
                    match message {
                        WorkerOutbound::SnapshotAt(snapshot, snapshot_rx) => {
                            return Ok((snapshot, snapshot_rx));
                        },
                        _ => {}
                    }
                },
                Err(_err) => { 
                    // add error conditional
                    return Err(RouterError::Timeout) 
                }
            }
        }
    }

    fn worker_index(&self, snapshot_id: u128) -> usize {
        let hash = self.hasher.hash_one(snapshot_id);
        let index = hash as usize % self.workers.len();

        index
    }
}
