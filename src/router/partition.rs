use ahash::RandomState;

use crate::batch::{prepare_inputs_by_snapshot, PreparedRequest, SnapshotInputBatch};
use crate::{InputLanes, SnapshotLanes};

pub(crate) struct WorkerInputBatch<IL> {
    pub(crate) snapshot_batches: Vec<(u128, SnapshotInputBatch<IL>)>,
    pub(crate) conservative_bytes: u64,
}

pub(crate) struct RoutePartitioner {
    worker_count: usize,
    hasher: RandomState,
}

impl RoutePartitioner {
    pub(crate) fn new(worker_count: usize) -> Self {
        Self::with_hasher(worker_count, RandomState::new())
    }

    pub(crate) fn with_hasher(worker_count: usize, hasher: RandomState) -> Self {
        assert!(worker_count > 0, "worker_count must be greater than zero");
        Self { worker_count, hasher }
    }

    pub(crate) fn worker_index(&self, snapshot_id: u128) -> usize {
        self.hasher.hash_one(snapshot_id) as usize % self.worker_count
    }

    pub(crate) fn partition_prepared_request<IL>(&self, request: PreparedRequest<IL>) -> Vec<Option<WorkerInputBatch<IL>>> {
        let mut worker_batches = Vec::with_capacity(self.worker_count);
        worker_batches.resize_with(self.worker_count, || None);

        for (snapshot_id, batch) in request.snapshots {
            let worker = worker_batches[self.worker_index(snapshot_id)]
                .get_or_insert_with(|| WorkerInputBatch { snapshot_batches: Vec::new(), conservative_bytes: 0 });
            worker.conservative_bytes = worker.conservative_bytes.saturating_add(batch.conservative_bytes);
            worker.snapshot_batches.push((snapshot_id, batch));
        }

        worker_batches
    }
}

/// Benchmark-only access to production route partitioning.
#[doc(hidden)]
pub struct RoutePartitionBenchmark {
    partitioner: RoutePartitioner,
}

impl RoutePartitionBenchmark {
    pub fn new(worker_count: usize) -> Self {
        Self { partitioner: RoutePartitioner::new(worker_count) }
    }

    pub fn prepare<SL, IL, I>(&self, inputs: I) -> PreparedRequest<IL>
    where
        SL: SnapshotLanes<Input = IL>,
        IL: InputLanes<SL>,
        I: IntoIterator<Item = IL>,
    {
        prepare_inputs_by_snapshot::<SL, IL, I>(inputs)
    }

    pub fn partition<IL>(&self, request: PreparedRequest<IL>) -> (usize, usize) {
        let worker_batches = self.partitioner.partition_prepared_request(request);
        let affected_workers = worker_batches.iter().flatten().count();
        let snapshot_batches = worker_batches.iter().flatten().map(|batch| batch.snapshot_batches.len()).sum();
        (affected_workers, snapshot_batches)
    }

    pub fn partition_storage<IL>(&self, request: PreparedRequest<IL>) -> (usize, usize) {
        let worker_batches = self.partitioner.partition_prepared_request(request);
        let initialized_workers = worker_batches.iter().flatten().count();
        (worker_batches.len(), initialized_workers)
    }
}
