use std::collections::hash_map::Entry;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ahash::{AHashMap, RandomState};
use crossbeam_channel::{Receiver, Sender};

use crate::memory::MemoryTracker;
use crate::{ApplyWrapper, EventRejection, InputLanes, SnapshotHistory, SnapshotLanes};

mod admission;

use admission::WorkerAdmission;

pub type SnapshotId = u128;

pub struct WorkerInput<IL> {
    pub snapshot_id: u128,
    pub input: IL,
}

/// An already-routed worker input batch used by the boundary benchmarks.
#[doc(hidden)]
pub struct WorkerApplyBatch<IL>(Vec<WorkerInput<IL>>);

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
    Inputs { inputs: Vec<WorkerInput<IL>>, completion: Completion<Vec<EventRejection>> },
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_parts(
        worker_inbound_tx: Sender<WorkerInbound<SL, IL>>,
        worker_inbound_rx: Receiver<WorkerInbound<SL, IL>>,
        _worker_index: usize,
        _worker_txs: Arc<Vec<Sender<WorkerInbound<SL, IL>>>>,
        _hasher: RandomState,
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
        let worker_txs = Arc::new(vec![worker_inbound_tx.clone()]);
        let worker = Worker::with_parts(
            worker_inbound_tx,
            worker_inbound_rx,
            0,
            worker_txs,
            RandomState::new(),
            MemoryTracker::new(memory_budget_bytes),
            lower_time_horizon_delta,
            (),
        );
        Self { worker }
    }
}

impl<SL, IL, C> WorkerApplyBenchmark<SL, IL, C>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
    C: ApplyWrapper<SL> + Send + 'static,
{
    pub fn prepare_batch<I>(&self, snapshot_id: u128, inputs: I) -> WorkerApplyBatch<IL>
    where
        I: IntoIterator<Item = IL>,
    {
        WorkerApplyBatch(inputs.into_iter().map(|input| WorkerInput { snapshot_id, input }).collect())
    }

    pub fn apply(&self, batch: WorkerApplyBatch<IL>) -> Vec<EventRejection> {
        let (response_tx, response_rx) = crossbeam_channel::unbounded();
        self.worker
            .worker_inbound_tx
            .send(WorkerInbound::Inputs { inputs: batch.0, completion: Completion::Respond(response_tx) })
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
    let mut admission = WorkerAdmission::new(lower_time_horizon_delta.clone());

    while is_running.load(Ordering::Relaxed) {
        let inbound = worker_inbound_rx.recv();

        match inbound {
            Ok(WorkerInbound::AdvanceTime { time: new_time, reply }) => {
                let identity_bytes_removed = admission.advance_to(new_time.clone());
                for history in history_by_id.values_mut() {
                    let bytes_delta = history.advance_with_context(new_time.clone(), &mut apply_context);
                    memory.apply_delta(bytes_delta);
                }
                memory.apply_delta(-(identity_bytes_removed as i64));
                let _ = reply.send(());
            }
            Ok(WorkerInbound::Inputs { inputs, completion }) => {
                let admitted = admission.admit(inputs, &memory);
                let history_bytes =
                    apply_inputs_to_histories(&mut history_by_id, lower_time_horizon_delta.clone(), &mut apply_context, admitted.inputs);
                let actual_bytes = (admitted.identity_bytes as i64).saturating_add(history_bytes);
                let reconciliation = actual_bytes.saturating_sub(admitted.reserved_bytes as i64);
                memory.apply_delta(reconciliation);
                complete(completion, admitted.rejections);
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

fn apply_inputs_to_histories<SL, IL, C>(
    history_by_id: &mut AHashMap<SnapshotId, SnapshotHistory<SL>>,
    lower_time_horizon_delta: SL::Time,
    apply_context: &mut C,
    inputs: Vec<WorkerInput<IL>>,
) -> i64
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    let mut bytes_delta = 0i64;
    let mut inputs_by_snapshot = AHashMap::<SnapshotId, Vec<IL>>::new();
    for routed_input in inputs {
        inputs_by_snapshot.entry(routed_input.snapshot_id).or_default().push(routed_input.input);
    }

    for (snapshot_id, inputs) in inputs_by_snapshot {
        let history = match history_by_id.entry(snapshot_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let (history, base_delta) =
                    SnapshotHistory::new_with_snapshot_id(snapshot_id, SL::Time::default(), lower_time_horizon_delta.clone());
                bytes_delta = bytes_delta.saturating_add(base_delta);
                entry.insert(history)
            }
        };
        bytes_delta = bytes_delta.saturating_add(history.apply_input_batch(inputs, apply_context));
    }
    bytes_delta
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
