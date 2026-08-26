use std::collections::hash_map::Entry;
use std::marker::PhantomData;
use std::ops::Bound;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ahash::{AHashMap, RandomState};
use crossbeam_channel::{Receiver, Sender};

use crate::{ApplyWrapper, ContimeKey, ContimeTime, EventRejection, Input, InputJournalEntry, InputLanes, SnapshotHistory, SnapshotLanes};

pub type SnapshotId = u128;

pub struct WorkerInput<IL> {
    pub snapshot_id: u128,
    pub input: IL,
}

pub enum Completion<T> {
    None,
    Respond(Sender<T>),
}

pub enum WorkerInbound<SL: SnapshotLanes, IL> {
    Inputs { inputs: Vec<WorkerInput<IL>>, completion: Completion<Vec<EventRejection>> },
    InputsInRange { start: Bound<SL::Time>, end: Bound<SL::Time>, reply: Sender<Vec<InputJournalEntry<IL>>> },
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
        memory_usage: Arc<AtomicU64>,
        lower_time_horizon_delta: SL::Time,
        apply_context: C,
    ) -> Self {
        let is_running = Arc::new(AtomicBool::new(true));
        let worker_running = Arc::clone(&is_running);
        let worker_memory_usage = Arc::clone(&memory_usage);
        let thread = thread::spawn(move || {
            handle_worker(worker_running, worker_inbound_rx, worker_memory_usage, lower_time_horizon_delta, apply_context);
        });

        Self { worker_inbound_tx, threads: vec![thread], is_running, _context: PhantomData }
    }
}

fn fetch_saturating_add_signed(atomic: &Arc<AtomicU64>, delta: i64, order: Ordering) {
    loop {
        let current = atomic.load(order);
        let new_value = if delta >= 0 { current.saturating_add(delta as u64) } else { current.saturating_sub((-delta) as u64) };
        if atomic.compare_exchange_weak(current, new_value, order, Ordering::Relaxed).is_ok() {
            break;
        }
    }
}

fn handle_worker<SL, IL, C>(
    is_running: Arc<AtomicBool>,
    worker_inbound_rx: Receiver<WorkerInbound<SL, IL>>,
    memory_usage: Arc<AtomicU64>,
    lower_time_horizon_delta: SL::Time,
    mut apply_context: C,
) where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    let mut history_by_id = AHashMap::<SnapshotId, SnapshotHistory<SL>>::new();
    let mut input_log = Vec::<InputJournalEntry<IL>>::new();

    while is_running.load(Ordering::Relaxed) {
        let inbound = worker_inbound_rx.recv();

        match inbound {
            Ok(WorkerInbound::AdvanceTime { time: new_time, reply }) => {
                for history in history_by_id.values_mut() {
                    let bytes_delta = history.advance_with_context(new_time.clone(), &mut apply_context);
                    fetch_saturating_add_signed(&memory_usage, bytes_delta, Ordering::Relaxed);
                }
                let drop_time = new_time.saturating_sub(lower_time_horizon_delta.clone());
                let bytes_removed = prune_input_log(&mut input_log, drop_time);
                fetch_saturating_add_signed(&memory_usage, -bytes_removed, Ordering::Relaxed);
                let _ = reply.send(());
            }
            Ok(WorkerInbound::Inputs { inputs, completion }) => {
                let bytes_delta = record_worker_inputs(&mut input_log, &inputs);
                fetch_saturating_add_signed(&memory_usage, bytes_delta, Ordering::Relaxed);
                apply_inputs_to_histories(&mut history_by_id, &memory_usage, lower_time_horizon_delta.clone(), &mut apply_context, inputs);
                complete(completion, Vec::new());
            }
            Ok(WorkerInbound::InputsInRange { start, end, reply }) => {
                let inputs = input_log.iter().filter(|entry| time_is_in_range(Input::time(&entry.input), &start, &end)).cloned().collect();
                let _ = reply.send(inputs);
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

fn record_worker_inputs<IL>(input_log: &mut Vec<InputJournalEntry<IL>>, inputs: &[WorkerInput<IL>]) -> i64
where
    IL: Input + Clone,
{
    let mut bytes_delta = 0i64;
    for routed_input in inputs {
        let key = ContimeKey::from_input(&routed_input.input);
        match input_log.binary_search_by_key(&key, |entry| ContimeKey::from_input(&entry.input)) {
            Ok(index) => {
                let entry = &mut input_log[index];
                if let Err(route_index) = entry.routed_snapshot_ids.binary_search(&routed_input.snapshot_id) {
                    entry.routed_snapshot_ids.insert(route_index, routed_input.snapshot_id);
                    bytes_delta = bytes_delta.saturating_add(size_of::<u128>() as i64);
                }
            }
            Err(index) => {
                let entry = InputJournalEntry { input: routed_input.input.clone(), routed_snapshot_ids: vec![routed_input.snapshot_id] };
                bytes_delta = bytes_delta.saturating_add(entry.conservative_size() as i64);
                input_log.insert(index, entry);
            }
        }
    }
    bytes_delta
}

fn prune_input_log<IL>(input_log: &mut Vec<InputJournalEntry<IL>>, time: IL::Time) -> i64
where
    IL: Input,
{
    let drop_key = ContimeKey { time, id: u128::MIN };
    let first_kept = input_log.partition_point(|entry| ContimeKey::from_input(&entry.input) < drop_key);
    let bytes_removed = input_log[..first_kept].iter().fold(0i64, |size, entry| size.saturating_add(entry.conservative_size() as i64));
    input_log.drain(..first_kept);
    bytes_removed
}

fn time_is_in_range<T: ContimeTime>(time: T, start: &Bound<T>, end: &Bound<T>) -> bool {
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

fn complete<T>(completion: Completion<T>, value: T) {
    if let Completion::Respond(response) = completion {
        let _ = response.send(value);
    }
}

fn apply_inputs_to_histories<SL, IL, C>(
    history_by_id: &mut AHashMap<SnapshotId, SnapshotHistory<SL>>,
    memory_usage: &Arc<AtomicU64>,
    lower_time_horizon_delta: SL::Time,
    apply_context: &mut C,
    inputs: Vec<WorkerInput<IL>>,
) where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
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
                fetch_saturating_add_signed(memory_usage, base_delta, Ordering::Relaxed);
                entry.insert(history)
            }
        };
        let bytes_delta = history.apply_input_batch(inputs, apply_context);
        fetch_saturating_add_signed(memory_usage, bytes_delta, Ordering::Relaxed);
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
