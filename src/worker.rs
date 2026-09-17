use std::collections::hash_map::Entry;
use std::marker::PhantomData;
use std::thread::{self, JoinHandle};

use ahash::AHashMap;
use crossbeam_channel::{Receiver, Sender};

use crate::batch::{prepare_inputs_by_snapshot, SnapshotInputBatch};
use crate::memory::MemoryTracker;
use crate::rejection::merge_event_rejections;
use crate::{ApplyWrapper, ContimeTime, EventRejection, EventRejectionReason, InputLanes, SnapshotHistory, SnapshotLanes};

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

pub enum WorkerInbound<SL: SnapshotLanes, IL> {
    Inputs { snapshot_batches: Vec<(u128, SnapshotInputBatch<IL>)>, conservative_bytes: u64, completion: Sender<Vec<EventRejection>> },
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
    _context: PhantomData<C>,
}

impl<SL, IL, C> Drop for Worker<SL, IL, C>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    fn drop(&mut self) {
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
        let thread = thread::spawn(move || {
            handle_worker(worker_inbound_rx, memory, lower_time_horizon_delta, apply_context);
        });

        Self { worker_inbound_tx, threads: vec![thread], _context: PhantomData }
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
    pub fn prepare_snapshot_batch<I>(&self, snapshot_id: u128, inputs: I) -> (u128, SnapshotInputBatch<IL>)
    where
        I: IntoIterator<Item = IL>,
    {
        let request = prepare_inputs_by_snapshot::<SL, IL, I>(inputs);
        assert_eq!(request.snapshots.len(), 1, "a direct worker fixture must prepare exactly one snapshot batch");
        let batch = request.snapshots.into_iter().next().expect("one prepared snapshot batch");
        assert_eq!(batch.0, snapshot_id, "the prepared worker batch routed to another snapshot");
        batch
    }

    pub fn apply_snapshot_batches(&self, snapshot_batches: Vec<(u128, SnapshotInputBatch<IL>)>) -> Vec<EventRejection> {
        let (response_tx, response_rx) = crossbeam_channel::unbounded();
        let conservative_bytes = snapshot_batches.iter().fold(0_u64, |total, (_, batch)| total.saturating_add(batch.conservative_bytes));
        self.worker
            .worker_inbound_tx
            .send(WorkerInbound::Inputs { snapshot_batches, conservative_bytes, completion: response_tx })
            .expect("benchmark worker remains connected");
        response_rx.into_iter().flatten().collect()
    }

    pub fn apply_inputs<I>(&self, snapshot_id: u128, inputs: I) -> Vec<EventRejection>
    where
        I: IntoIterator<Item = IL>,
    {
        let batch = self.prepare_snapshot_batch(snapshot_id, inputs);
        self.apply_snapshot_batches(vec![batch])
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

    while let Ok(inbound) = worker_inbound_rx.recv() {
        match inbound {
            WorkerInbound::AdvanceTime { time: new_time, reply } => {
                current_time = new_time.clone();
                for history in history_by_id.values_mut() {
                    let bytes_delta = history.advance_with_context(new_time.clone(), &mut apply_context);
                    memory.apply_delta(bytes_delta);
                }
                let _ = reply.send(());
            }
            WorkerInbound::Inputs { snapshot_batches, conservative_bytes, completion } => {
                if !memory.try_reserve(conservative_bytes) {
                    complete(completion, memory_full_rejections_for_worker(&snapshot_batches));
                    continue;
                }

                let result = apply_snapshot_batches(
                    snapshot_batches,
                    &mut history_by_id,
                    current_time.clone(),
                    lower_time_horizon_delta.clone(),
                    &mut apply_context,
                    &memory,
                );
                memory.reconcile_reservation(conservative_bytes, result.actual_delta);
                complete(completion, result.rejections);
            }
            WorkerInbound::SnapshotsAt { snapshot_requests, time, reply } => {
                let mut results = Vec::with_capacity(snapshot_requests.len());
                for (position, snapshot_id) in snapshot_requests {
                    let snapshot = history_by_id
                        .get(&snapshot_id)
                        .and_then(|history| history.snapshot_only_at_with_context(time.clone(), &mut apply_context));
                    results.push((position, snapshot));
                }
                let _ = reply.send(results);
            }
            WorkerInbound::Shutdown => break,
        }
    }
}

struct WorkerApplyResult {
    actual_delta: i64,
    rejections: Vec<EventRejection>,
}

fn apply_snapshot_batches<SL, IL, C>(
    snapshot_batches: Vec<(u128, SnapshotInputBatch<IL>)>,
    history_by_id: &mut AHashMap<SnapshotId, SnapshotHistory<SL>>,
    current_time: SL::Time,
    lower_time_horizon_delta: SL::Time,
    apply_context: &mut C,
    memory: &MemoryTracker,
) -> WorkerApplyResult
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    let mut actual_delta = 0_i64;
    let mut rejections = Vec::new();
    for (snapshot_id, batch) in snapshot_batches {
        if let Some(stale_rejections) = stale_unseen_batch_rejections(
            history_by_id.contains_key(&snapshot_id),
            &batch,
            current_time.clone(),
            lower_time_horizon_delta.clone(),
        ) {
            merge_event_rejections(&mut rejections, stale_rejections);
            continue;
        }
        let history = match history_by_id.entry(snapshot_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let (history, base_delta) =
                    SnapshotHistory::new_with_snapshot_id(snapshot_id, current_time.clone(), lower_time_horizon_delta.clone());
                actual_delta = actual_delta.saturating_add(base_delta);
                entry.insert(history)
            }
        };
        let result = history.apply_routed_input_batch_with_memory(batch.inputs, apply_context, memory);
        actual_delta = actual_delta.saturating_add(result.bytes_delta);
        merge_event_rejections(&mut rejections, result.rejections);
    }
    WorkerApplyResult { actual_delta, rejections }
}

fn stale_unseen_batch_rejections<SL, IL>(
    history_exists: bool,
    batch: &SnapshotInputBatch<IL>,
    current_time: SL::Time,
    lower_time_horizon_delta: SL::Time,
) -> Option<Vec<EventRejection>>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
{
    if history_exists {
        return None;
    }
    let earliest_retained_time = current_time.saturating_sub(lower_time_horizon_delta);
    batch
        .inputs
        .iter()
        .all(|input| input.time() < earliest_retained_time.clone())
        .then(|| batch.inputs.iter().map(|input| EventRejection::new(input.id(), EventRejectionReason::BeforeHistoryHorizon)).collect())
}

fn memory_full_rejections_for_worker<IL>(snapshot_batches: &[(u128, SnapshotInputBatch<IL>)]) -> Vec<EventRejection>
where
    IL: crate::Input,
{
    let mut input_ids = Vec::new();
    for (_, batch) in snapshot_batches {
        batch.unique_input_ids(&mut input_ids);
    }
    input_ids.sort_unstable();
    input_ids.dedup();
    input_ids.into_iter().map(|event_id| EventRejection::new(event_id, EventRejectionReason::MemoryFull)).collect()
}

fn complete(completion: Sender<Vec<EventRejection>>, rejections: Vec<EventRejection>) {
    if !rejections.is_empty() {
        let _ = completion.send(rejections);
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::{bounded, TryRecvError};

    use super::{complete, stale_unseen_batch_rejections};
    use crate::batch::prepare_inputs_by_snapshot;
    use crate::{EventRejection, EventRejectionReason, TestEvent, TestInputLanes, TestSnapshotLanes};

    #[test]
    fn rejection_completion_sends_the_rejections_before_disconnect() {
        let (response_tx, response_rx) = bounded(2);
        let expected = vec![EventRejection::new(7, EventRejectionReason::MemoryFull)];

        complete(response_tx, expected.clone());

        assert_eq!(response_rx.recv().unwrap(), expected);
        assert_eq!(response_rx.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn successful_completion_sends_no_value_before_disconnect() {
        let (response_tx, response_rx) = bounded(1);

        complete(response_tx, Vec::<EventRejection>::new());

        assert_eq!(response_rx.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn stale_only_batch_for_unseen_snapshot_is_rejected_before_history_creation() {
        let request = prepare_inputs_by_snapshot::<TestSnapshotLanes, TestInputLanes, _>([
            TestEvent::Positive(7, 49, 11, 1).into(),
            TestEvent::Positive(7, 20, 12, 1).into(),
        ]);
        let batch = request.snapshots.get(&7).unwrap();

        assert_eq!(
            stale_unseen_batch_rejections(false, batch, 100, 50),
            Some(vec![
                EventRejection::new(11, EventRejectionReason::BeforeHistoryHorizon),
                EventRejection::new(12, EventRejectionReason::BeforeHistoryHorizon),
            ])
        );
    }
}
