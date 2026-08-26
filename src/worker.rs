use std::collections::hash_map::Entry;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ahash::AHashMap;
use crossbeam_channel::{Receiver, Sender};

use crate::batch::{group_inputs_by_snapshot, memory_full_rejections, total_conservative_bytes, SnapshotInputBatch};
use crate::memory::MemoryTracker;
use crate::rejection::merge_event_rejections;
use crate::{ApplyWrapper, EventRejection, InputLanes, SnapshotHistory, SnapshotLanes};

pub type SnapshotId = u128;

/// Benchmark-only access to one production worker without the router.
#[doc(hidden)]
pub struct WorkerApplyBenchmark<SL, IL, C = ()>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    worker: Worker<SL, IL, C>,
}

pub enum Completion<T> {
    None,
    Respond(Sender<T>),
}

pub enum WorkerInbound<SL: SnapshotLanes, IL> {
    Inputs { snapshot_batches: Vec<SnapshotInputBatch<IL>>, conservative_bytes: u64, completion: Completion<Vec<EventRejection>> },
    SnapshotsAt { snapshot_requests: Vec<(usize, u128)>, time: SL::Time, reply: Sender<Vec<(usize, Option<SL>)>> },
    AdvanceTime { time: SL::Time, reply: Sender<()> },
    Shutdown,
}

pub struct Worker<SL, IL, C = ()>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    pub worker_inbound_tx: Sender<WorkerInbound<SL, IL>>,
    threads: Vec<JoinHandle<()>>,
    is_running: Arc<AtomicBool>,
    _context: PhantomData<C>,
}

impl<SL, IL, C> Drop for Worker<SL, IL, C>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);
        let _ = self.worker_inbound_tx.send(WorkerInbound::<SL, IL>::Shutdown);

        for thread in self.threads.drain(..) {
            if let Err(error) = thread.join() {
                eprintln!("contime worker thread panicked: {:?}", error);
            }
        }
    }
}

impl<SL, IL, C> Worker<SL, IL, C>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
    C: ApplyWrapper<SL> + Send + 'static,
{
    pub(crate) fn with_parts(
        worker_inbound_tx: Sender<WorkerInbound<SL, IL>>,
        worker_inbound_rx: Receiver<WorkerInbound<SL, IL>>,
        memory: MemoryTracker,
        lower_time_horizon_delta: SL::Time,
        apply_context: C,
    ) -> Self {
        let is_running = Arc::new(AtomicBool::new(true));
        let worker_running = Arc::clone(&is_running);
        let thread = thread::spawn(move || {
            handle_worker(worker_running, worker_inbound_rx, memory, lower_time_horizon_delta, apply_context);
        });

        Self { worker_inbound_tx, threads: vec![thread], is_running, _context: PhantomData }
    }
}

impl<SL, IL> WorkerApplyBenchmark<SL, IL>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
{
    pub fn new(memory_budget_bytes: u64, lower_time_horizon_delta: SL::Time) -> Self {
        let (worker_inbound_tx, worker_inbound_rx) = crossbeam_channel::unbounded();
        let worker =
            Worker::with_parts(worker_inbound_tx, worker_inbound_rx, MemoryTracker::new(memory_budget_bytes), lower_time_horizon_delta, ());
        Self { worker }
    }
}

impl<SL, IL, C> WorkerApplyBenchmark<SL, IL, C>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
    C: ApplyWrapper<SL> + Send + 'static,
{
    pub fn prepare_snapshot_batch<I>(&self, snapshot_id: u128, inputs: I) -> SnapshotInputBatch<IL>
    where
        I: IntoIterator<Item = IL>,
    {
        let mut batches = group_inputs_by_snapshot::<SL, IL, I>(inputs);
        assert_eq!(batches.len(), 1, "a direct worker fixture must prepare exactly one snapshot batch");
        let batch = batches.pop().expect("one prepared snapshot batch");
        assert_eq!(batch.snapshot_id, snapshot_id, "the prepared worker batch routed to another snapshot");
        batch
    }

    pub fn apply_snapshot_batches(&self, snapshot_batches: Vec<SnapshotInputBatch<IL>>) -> Vec<EventRejection> {
        let (response_tx, response_rx) = crossbeam_channel::unbounded();
        let conservative_bytes = total_conservative_bytes(&snapshot_batches);
        self.worker
            .worker_inbound_tx
            .send(WorkerInbound::Inputs { snapshot_batches, conservative_bytes, completion: Completion::Respond(response_tx) })
            .expect("benchmark worker remains connected");
        response_rx.recv().expect("benchmark worker returns one completion")
    }

    pub fn warm_up(&self, time: SL::Time) {
        let (response_tx, response_rx) = crossbeam_channel::bounded(1);
        self.worker
            .worker_inbound_tx
            .send(WorkerInbound::AdvanceTime { time, reply: response_tx })
            .expect("benchmark worker remains connected");
        response_rx.recv().expect("benchmark worker completes warm-up");
    }

    pub fn snapshot_at(&self, snapshot_id: u128, time: SL::Time) -> Option<SL> {
        let (response_tx, response_rx) = crossbeam_channel::bounded(1);
        self.worker
            .worker_inbound_tx
            .send(WorkerInbound::SnapshotsAt { snapshot_requests: vec![(0, snapshot_id)], time, reply: response_tx })
            .expect("benchmark worker remains connected");
        response_rx
            .recv()
            .expect("benchmark worker returns one query response")
            .into_iter()
            .next()
            .and_then(|(_position, snapshot)| snapshot)
    }
}

fn handle_worker<SL, IL, C>(
    is_running: Arc<AtomicBool>,
    worker_inbound_rx: Receiver<WorkerInbound<SL, IL>>,
    memory: MemoryTracker,
    lower_time_horizon_delta: SL::Time,
    mut apply_context: C,
) where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    let mut history_by_id = AHashMap::<SnapshotId, SnapshotHistory<SL>>::new();
    let mut current_time = SL::Time::default();

    while is_running.load(Ordering::Relaxed) {
        let inbound = worker_inbound_rx.recv();

        match inbound {
            Ok(WorkerInbound::AdvanceTime { time: new_time, reply }) => {
                current_time = new_time.clone();
                for history in history_by_id.values_mut() {
                    let bytes_delta = history.advance_with_context(new_time.clone(), &mut apply_context);
                    memory.apply_delta(bytes_delta);
                }
                let _ = reply.send(());
            }
            Ok(WorkerInbound::Inputs { snapshot_batches, conservative_bytes, completion }) => {
                let existing_replay_bytes = snapshot_batches.iter().fold(0_u64, |total, batch| {
                    total.saturating_add(history_by_id.get(&batch.snapshot_id).map_or(0, SnapshotHistory::conservative_replay_reservation))
                });
                let reservation_bytes = conservative_bytes.saturating_add(existing_replay_bytes);
                if !memory.try_reserve(reservation_bytes) {
                    complete(completion, memory_full_rejections(&snapshot_batches));
                    continue;
                }

                let mut actual_delta = 0_i64;
                let mut rejections = Vec::new();
                for batch in snapshot_batches {
                    let history = match history_by_id.entry(batch.snapshot_id) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            let (history, base_delta) = SnapshotHistory::new_with_snapshot_id(
                                batch.snapshot_id,
                                current_time.clone(),
                                lower_time_horizon_delta.clone(),
                            );
                            actual_delta = actual_delta.saturating_add(base_delta);
                            entry.insert(history)
                        }
                    };
                    let result = history.apply_routed_input_batch(batch.inputs, &mut apply_context);
                    actual_delta = actual_delta.saturating_add(result.bytes_delta);
                    merge_event_rejections(&mut rejections, result.rejections);
                }
                memory.reconcile_reservation(reservation_bytes, actual_delta);
                complete(completion, rejections);
            }
            Ok(WorkerInbound::SnapshotsAt { snapshot_requests, time, reply }) => {
                let mut results = Vec::with_capacity(snapshot_requests.len());
                for (position, snapshot_id) in snapshot_requests {
                    let snapshot = history_by_id
                        .get(&snapshot_id)
                        .and_then(|history| history.snapshot_only_at_with_context(time.clone(), &mut apply_context));
                    results.push((position, snapshot));
                }
                let _ = reply.send(results);
            }
            Ok(WorkerInbound::Shutdown) | Err(_) => return,
        }
    }
}

fn complete<T>(completion: Completion<T>, value: T) {
    if let Completion::Respond(response) = completion {
        let _ = response.send(value);
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::{bounded, TryRecvError};

    use super::{complete, Completion};
    use crate::{EventRejection, EventRejectionReason};

    #[test]
    fn responding_completion_sends_exactly_one_batch_result() {
        let (response_tx, response_rx) = bounded(2);
        let expected = vec![EventRejection::new(7, EventRejectionReason::MemoryFull)];

        complete(Completion::Respond(response_tx.clone()), expected.clone());

        assert_eq!(response_rx.recv().unwrap(), expected);
        assert_eq!(response_rx.try_recv(), Err(TryRecvError::Empty));
    }
}
