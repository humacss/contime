use std::marker::PhantomData;
use std::ops::Bound;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use ahash::RandomState;
use crossbeam_channel::{unbounded, Sender};

use crate::worker::{Completion, WorkerInput};
use crate::{ApplyWrapper, EventRejection, InputJournalEntry, InputLanes, SnapshotLanes, Worker, WorkerInbound};

#[derive(Debug)]
pub enum RouterError {
    WorkerUnavailable,
}

pub struct Router<SL, IL, C = (), G = ()>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    C: ApplyWrapper<SL>,
{
    hasher: RandomState,
    workers: Vec<Worker<SL, IL, C>>,
    global_context: Arc<G>,
    _context: PhantomData<C>,
}

impl<SL, IL> Router<SL, IL, (), ()>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
{
    pub fn new(worker_count: usize, memory_budget_bytes: u64) -> Self {
        Self::new_with_apply_context(worker_count, memory_budget_bytes, ())
    }

    pub fn with_history_horizon(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: SL::Time) -> Self {
        Self::with_history_horizon_and_apply_context(worker_count, memory_budget_bytes, lower_time_horizon_delta, ())
    }
}

impl<SL, IL, C> Router<SL, IL, C, ()>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
    C: ApplyWrapper<SL> + Clone + Send + 'static,
{
    pub fn new_with_apply_context(worker_count: usize, memory_budget_bytes: u64, apply_context: C) -> Self {
        Self::new_with_contexts(worker_count, memory_budget_bytes, (), move |_, _| apply_context.clone())
    }

    pub fn with_history_horizon_and_apply_context(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: SL::Time,
        apply_context: C,
    ) -> Self {
        Self::with_history_horizon_and_contexts(worker_count, memory_budget_bytes, lower_time_horizon_delta, (), move |_, _| {
            apply_context.clone()
        })
    }
}

impl<SL, IL, C, G> Router<SL, IL, C, G>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
    C: ApplyWrapper<SL> + Send + 'static,
    G: Send + Sync + 'static,
{
    pub fn new_with_contexts<F>(worker_count: usize, memory_budget_bytes: u64, global_context: G, make_apply_context: F) -> Self
    where
        F: FnMut(usize, Arc<G>) -> C,
    {
        Self::with_history_horizon_and_contexts(worker_count, memory_budget_bytes, SL::Time::default(), global_context, make_apply_context)
    }

    pub fn with_history_horizon_and_contexts<F>(
        worker_count: usize,
        memory_budget_bytes: u64,
        lower_time_horizon_delta: SL::Time,
        global_context: G,
        mut make_apply_context: F,
    ) -> Self
    where
        F: FnMut(usize, Arc<G>) -> C,
    {
        assert!(worker_count > 0, "worker_count must be greater than zero");

        let hasher = RandomState::new();
        let global_context = Arc::new(global_context);
        let memory_budget = Arc::new(AtomicU64::new(memory_budget_bytes));
        let memory_usage = Arc::new(AtomicU64::new(0));
        let mut worker_txs = Vec::<Sender<WorkerInbound<SL, IL>>>::with_capacity(worker_count);
        let mut worker_rxs = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let (tx, rx) = unbounded::<WorkerInbound<SL, IL>>();
            worker_txs.push(tx);
            worker_rxs.push(rx);
        }
        let worker_txs = Arc::new(worker_txs);

        let mut workers = Vec::with_capacity(worker_count);
        for (worker_index, rx) in worker_rxs.into_iter().enumerate() {
            workers.push(Worker::<SL, IL, C>::with_parts(
                worker_txs[worker_index].clone(),
                rx,
                worker_index,
                Arc::clone(&worker_txs),
                hasher.clone(),
                Arc::clone(&memory_budget),
                Arc::clone(&memory_usage),
                lower_time_horizon_delta.clone(),
                make_apply_context(worker_index, Arc::clone(&global_context)),
            ));
        }

        Self { hasher, workers, global_context, _context: PhantomData }
    }

    pub fn global_context(&self) -> Arc<G> {
        Arc::clone(&self.global_context)
    }

    pub(crate) fn dispatch_inputs<I>(&self, inputs: I, response: Option<&Sender<Vec<EventRejection>>>) -> Result<usize, RouterError>
    where
        I: IntoIterator<Item = IL>,
    {
        let mut worker_inputs = Vec::with_capacity(self.workers.len());
        worker_inputs.resize_with(self.workers.len(), Vec::new);
        let mut routed_snapshots = Vec::<(u128, usize)>::new();

        for input in inputs {
            routed_snapshots.clear();
            input.visit_snapshot_ids(&mut |snapshot_id| routed_snapshots.push((snapshot_id, self.worker_index(snapshot_id))));
            if routed_snapshots.is_empty() {
                continue;
            }
            let route_count = routed_snapshots.len();
            let mut input = Some(input);
            for (route_position, &(snapshot_id, worker_index)) in routed_snapshots.iter().enumerate() {
                let routed_input = if route_position + 1 == route_count {
                    input.take().expect("the final route owns the input")
                } else {
                    input.as_ref().expect("earlier routes retain the input").clone()
                };
                worker_inputs[worker_index].push(WorkerInput { snapshot_id, input: routed_input });
            }
        }

        let mut affected_workers = 0;
        for (worker_index, inputs) in worker_inputs.into_iter().enumerate() {
            if inputs.is_empty() {
                continue;
            }
            let completion = response.map_or(Completion::None, |response| Completion::Respond(response.clone()));
            self.workers[worker_index]
                .worker_inbound_tx
                .send(WorkerInbound::Inputs { inputs, completion })
                .map_err(|_| RouterError::WorkerUnavailable)?;
            affected_workers += 1;
        }
        Ok(affected_workers)
    }

    pub(crate) fn dispatch_inspection(
        &self,
        start: Bound<SL::Time>,
        end: Bound<SL::Time>,
        response: &Sender<Vec<InputJournalEntry<IL>>>,
    ) -> Result<usize, RouterError> {
        for worker in &self.workers {
            worker
                .worker_inbound_tx
                .send(WorkerInbound::InputsInRange { start: start.clone(), end: end.clone(), reply: response.clone() })
                .map_err(|_| RouterError::WorkerUnavailable)?;
        }
        Ok(self.workers.len())
    }

    pub(crate) fn dispatch_query(
        &self,
        time: SL::Time,
        positioned_snapshot_ids: &[(usize, u128)],
        response: &Sender<Vec<(usize, Option<SL>)>>,
    ) -> Result<usize, RouterError> {
        let mut requests_by_worker = Vec::with_capacity(self.workers.len());
        requests_by_worker.resize_with(self.workers.len(), Vec::new);
        for &(position, snapshot_id) in positioned_snapshot_ids {
            requests_by_worker[self.worker_index(snapshot_id)].push((position, snapshot_id));
        }

        let mut affected_workers = 0;
        for (worker_index, snapshot_requests) in requests_by_worker.into_iter().enumerate() {
            if snapshot_requests.is_empty() {
                continue;
            }
            self.workers[worker_index]
                .worker_inbound_tx
                .send(WorkerInbound::SnapshotsAt { snapshot_requests, time: time.clone(), reply: response.clone() })
                .map_err(|_| RouterError::WorkerUnavailable)?;
            affected_workers += 1;
        }
        Ok(affected_workers)
    }

    pub(crate) fn dispatch_advance(&self, time: SL::Time, response: &Sender<()>) -> Result<usize, RouterError> {
        for worker in &self.workers {
            worker
                .worker_inbound_tx
                .send(WorkerInbound::AdvanceTime { time: time.clone(), reply: response.clone() })
                .map_err(|_| RouterError::WorkerUnavailable)?;
        }
        Ok(self.workers.len())
    }

    fn worker_index(&self, snapshot_id: u128) -> usize {
        self.hasher.hash_one(snapshot_id) as usize % self.workers.len()
    }
}

#[cfg(test)]
mod tests;
