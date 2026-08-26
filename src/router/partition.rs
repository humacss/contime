use ahash::RandomState;

use crate::worker::WorkerInput;
use crate::{InputLanes, SnapshotLanes};

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

    pub(crate) fn partition<SL, IL, I>(&self, inputs: I) -> Vec<Vec<WorkerInput<IL>>>
    where
        SL: SnapshotLanes<Input = IL>,
        IL: InputLanes<SL>,
        I: IntoIterator<Item = IL>,
    {
        let inputs = inputs.into_iter();
        let input_capacity = inputs.size_hint().0;
        let bucket_capacity = input_capacity.div_ceil(self.worker_count);
        let mut worker_inputs = Vec::with_capacity(self.worker_count);
        worker_inputs.resize_with(self.worker_count, || Vec::with_capacity(bucket_capacity));
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

        worker_inputs
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

    pub fn partition<SL, IL, I>(&self, inputs: I) -> (usize, usize)
    where
        SL: SnapshotLanes<Input = IL>,
        IL: InputLanes<SL>,
        I: IntoIterator<Item = IL>,
    {
        let worker_inputs = self.partitioner.partition::<SL, IL, I>(inputs);
        let affected_workers = worker_inputs.iter().filter(|inputs| !inputs.is_empty()).count();
        let routed_events = worker_inputs.iter().map(Vec::len).sum();
        (affected_workers, routed_events)
    }
}
