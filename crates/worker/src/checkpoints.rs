use ahash::AHashMap;

use crate::memory::Memory;
use crate::types::{complete_snapshot, Checkpoints, Completion, SnapshotSlot};

pub(crate) fn update_snapshot<S, K, C, R>(
    snapshot_id: u128,
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    memory: &mut Memory,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
) where
    K: Checkpoints<S>,
    C: Completion<R>,
{
    let slot = snapshots.get_mut(&snapshot_id).expect("dirty schedule referenced a missing event store");

    if slot.checkpoints.is_none() {
        let created = K::create(snapshot_id, checkpoints_config, memory.remaining());
        memory.apply_delta(created.retained_bytes_delta);
        slot.checkpoints = Some(created.checkpoints);
    }

    let checkpoints = slot.checkpoints.as_mut().expect("checkpoint store was not initialized");
    let result = checkpoints.update(&slot.events, checkpoints_context, memory.remaining());
    memory.apply_delta(result.retained_bytes_delta);

    for request in slot.waiters.drain(..) {
        complete_snapshot(request);
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use ahash::AHashMap;
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, TryRecvError};

    use super::update_snapshot;
    use crate::memory::Memory;
    use crate::types::{new_request, register_waiter, SnapshotSlot};
    use crate::{CheckpointResult, Checkpoints, CheckpointsCreated, WorkerRejection};

    struct TestEvents(Vec<u128>);

    #[derive(Default)]
    struct TestCheckpoints {
        event_count: usize,
    }

    impl Checkpoints<TestEvents> for TestCheckpoints {
        type Config = ();
        type Context = Vec<usize>;

        fn create(_snapshot_id: u128, _config: &(), _limit: u64) -> CheckpointsCreated<Self> {
            CheckpointsCreated { checkpoints: Self::default(), retained_bytes_delta: 8 }
        }

        fn update(&mut self, events: &TestEvents, context: &mut Vec<usize>, _limit: u64) -> CheckpointResult {
            self.event_count = events.0.len();
            context.push(self.event_count);
            CheckpointResult { retained_bytes_delta: 16 }
        }
    }

    fn snapshot() -> (
        AHashMap<u128, SnapshotSlot<TestEvents, TestCheckpoints, crossbeam_channel::Sender<Vec<WorkerRejection<()>>>, ()>>,
        crossbeam_channel::Receiver<Vec<WorkerRejection<()>>>,
    ) {
        let (completion, responses) = unbounded();
        let request = new_request(completion);
        let mut slot = SnapshotSlot { events: TestEvents((0..1_000).collect()), checkpoints: None, waiters: Vec::new() };
        register_waiter(&mut slot, &request);
        let mut snapshots = AHashMap::new();
        snapshots.insert(7, slot);
        (snapshots, responses)
    }

    #[test]
    fn checkpoint_update_reads_events_and_completes_snapshot_waiters() {
        let (mut snapshots, responses) = snapshot();
        let mut memory = Memory::new(1_000);
        let mut context = Vec::new();

        update_snapshot(7, &mut snapshots, &mut memory, &(), &mut context);

        assert_eq!(context, vec![1_000]);
        assert_eq!(snapshots.get(&7).unwrap().checkpoints.as_ref().unwrap().event_count, 1_000);
        assert_eq!(memory.used(), 24);
        assert_eq!(responses.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_checkpoints() {
        let mut criterion = Criterion::default();
        criterion.bench_function("worker/checkpoints/1000_events", |bencher| {
            bencher.iter_batched(
                || snapshot().0,
                |mut snapshots| {
                    let mut memory = Memory::new(1_000_000);
                    let mut context = Vec::new();
                    update_snapshot(7, &mut snapshots, &mut memory, &(), &mut context);
                    black_box((snapshots, memory, context));
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
