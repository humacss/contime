use ahash::RandomState;

use crate::batch::{group_inputs_by_snapshot, SnapshotInputBatch};
use crate::{InputLanes, SnapshotLanes};

pub(crate) struct WorkerInputBatch<IL> {
    pub(crate) snapshot_batches: Vec<SnapshotInputBatch<IL>>,
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

    pub(crate) fn partition_snapshot_batches<IL>(&self, batches: Vec<SnapshotInputBatch<IL>>) -> Vec<WorkerInputBatch<IL>> {
        let batch_capacity = batches.len().div_ceil(self.worker_count);
        let mut worker_batches = Vec::with_capacity(self.worker_count);
        worker_batches.resize_with(self.worker_count, || WorkerInputBatch {
            snapshot_batches: Vec::with_capacity(batch_capacity),
            conservative_bytes: 0,
        });

        for batch in batches {
            let worker = &mut worker_batches[self.worker_index(batch.snapshot_id)];
            worker.conservative_bytes = worker.conservative_bytes.saturating_add(batch.conservative_bytes);
            worker.snapshot_batches.push(batch);
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

    pub fn prepare<SL, IL, I>(&self, inputs: I) -> Vec<SnapshotInputBatch<IL>>
    where
        SL: SnapshotLanes<Input = IL>,
        IL: InputLanes<SL>,
        I: IntoIterator<Item = IL>,
    {
        group_inputs_by_snapshot::<SL, IL, I>(inputs)
    }

    pub fn partition<IL>(&self, batches: Vec<SnapshotInputBatch<IL>>) -> (usize, usize) {
        let worker_batches = self.partitioner.partition_snapshot_batches(batches);
        let affected_workers = worker_batches.iter().filter(|batch| !batch.snapshot_batches.is_empty()).count();
        let snapshot_batches = worker_batches.iter().map(|batch| batch.snapshot_batches.len()).sum();
        (affected_workers, snapshot_batches)
    }
}
