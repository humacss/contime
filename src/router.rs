use std::marker::PhantomData;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use ahash::RandomState;
use crossbeam_channel::{unbounded, Sender};

mod partition;

pub use partition::RoutePartitionBenchmark;
use partition::RoutePartitioner;

use crate::worker::Completion;
use crate::{ApplyWrapper, EventRejection, InputLanes, SnapshotLanes, Worker, WorkerInbound};

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
    partitioner: RoutePartitioner,
    workers: Vec<Worker<SL, IL, C>>,
    global_context: Arc<G>,
    _context: PhantomData<C>,
}

/// Benchmark-only access to production routing plus worker completion.
#[doc(hidden)]
pub struct RouterApplyBenchmark<SL, IL>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
{
    router: Router<SL, IL>,
}

impl<SL, IL> RouterApplyBenchmark<SL, IL>
where
    SL: SnapshotLanes<Input = IL> + 'static,
    IL: InputLanes<SL> + Send + 'static,
{
    pub fn new(worker_count: usize, memory_budget_bytes: u64, lower_time_horizon_delta: SL::Time) -> Self {
        Self { router: Router::with_history_horizon(worker_count, memory_budget_bytes, lower_time_horizon_delta) }
    }

    pub fn apply<I>(&self, inputs: I) -> Vec<EventRejection>
    where
        I: IntoIterator<Item = IL>,
    {
        let (response_tx, response_rx) = crossbeam_channel::unbounded();
        let expected = self.router.dispatch_inputs(inputs, Some(&response_tx)).expect("benchmark router workers remain connected");
        drop(response_tx);
        let mut rejections = Vec::new();
        for _ in 0..expected {
            rejections.extend(response_rx.recv().expect("each benchmark worker completes its batch"));
        }
        rejections.sort_unstable();
        rejections.dedup();
        rejections
    }

    pub fn warm_up(&self, time: SL::Time) {
        let (response_tx, response_rx) = crossbeam_channel::unbounded();
        let expected = self.router.dispatch_advance(time, &response_tx).expect("benchmark router workers remain connected");
        drop(response_tx);
        for _ in 0..expected {
            response_rx.recv().expect("each benchmark worker completes warm-up");
        }
    }

    pub fn snapshot_at(&self, snapshot_id: u128, time: SL::Time) -> Option<SL> {
        let (response_tx, response_rx) = crossbeam_channel::bounded(1);
        let expected =
            self.router.dispatch_query(time, &[(0, snapshot_id)], &response_tx).expect("benchmark router workers remain connected");
        drop(response_tx);
        (0..expected)
            .flat_map(|_| response_rx.recv().expect("affected benchmark worker returns its query response"))
            .find_map(|(_position, snapshot)| snapshot)
    }
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
        let partitioner = RoutePartitioner::with_hasher(worker_count, hasher.clone());
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

        Self { partitioner, workers, global_context, _context: PhantomData }
    }

    pub fn global_context(&self) -> Arc<G> {
        Arc::clone(&self.global_context)
    }

    pub(crate) fn dispatch_inputs<I>(&self, inputs: I, response: Option<&Sender<Vec<EventRejection>>>) -> Result<usize, RouterError>
    where
        I: IntoIterator<Item = IL>,
    {
        let worker_inputs = self.partitioner.partition::<SL, IL, I>(inputs);

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
        self.partitioner.worker_index(snapshot_id)
    }
}

#[cfg(test)]
mod tests;
