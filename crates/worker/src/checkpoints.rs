use ahash::AHashMap;

use crate::types::{complete_snapshot, Checkpoints, Completion, SnapshotSlot};

pub(crate) fn update_snapshot<S, K, C, R>(
    snapshot_id: u128,
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
) where
    K: Checkpoints<S>,
    C: Completion<R>,
{
    let slot = snapshots.get_mut(&snapshot_id).expect("dirty schedule referenced a missing event store");

    if slot.checkpoints.is_none() {
        slot.checkpoints = Some(K::create(snapshot_id, checkpoints_config));
    }

    let checkpoints = slot.checkpoints.as_mut().expect("checkpoint store was not initialized");
    checkpoints.update(&mut slot.events, checkpoints_context);

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
    use crate::types::{new_request, register_waiter, SnapshotSlot};
    use crate::Checkpoints;

    struct TestEvents(Vec<u128>);

    struct AcknowledgingCheckpoints;

    impl Checkpoints<TestEvents> for AcknowledgingCheckpoints {
        type Config = ();
        type Context = ();

        fn create(_snapshot_id: u128, _config: &()) -> Self {
            Self
        }

        fn update(&mut self, events: &mut TestEvents, _context: &mut ()) {
            events.0.clear();
        }
    }

    #[derive(Default)]
    struct TestCheckpoints {
        event_count: usize,
    }

    impl Checkpoints<TestEvents> for TestCheckpoints {
        type Config = ();
        type Context = Vec<usize>;

        fn create(_snapshot_id: u128, _config: &()) -> Self {
            Self::default()
        }

        fn update(&mut self, events: &mut TestEvents, context: &mut Vec<usize>) {
            self.event_count = events.0.len();
            context.push(self.event_count);
        }
    }

    fn snapshot() -> (
        AHashMap<u128, SnapshotSlot<TestEvents, TestCheckpoints, crossbeam_channel::Sender<Vec<()>>, ()>>,
        crossbeam_channel::Receiver<Vec<()>>,
    ) {
        let (completion, responses) = unbounded::<Vec<()>>();
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
        let mut context = Vec::new();

        update_snapshot(7, &mut snapshots, &(), &mut context);

        assert_eq!(context, vec![1_000]);
        assert_eq!(snapshots.get(&7).unwrap().checkpoints.as_ref().unwrap().event_count, 1_000);
        assert_eq!(responses.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn checkpoint_update_can_acknowledge_mutable_event_history() {
        let (completion, responses) = unbounded::<Vec<()>>();
        let request = new_request(completion);
        let mut slot =
            SnapshotSlot { events: TestEvents(vec![1, 2, 3]), checkpoints: None::<AcknowledgingCheckpoints>, waiters: Vec::new() };
        register_waiter(&mut slot, &request);
        let mut snapshots = AHashMap::new();
        snapshots.insert(7, slot);

        update_snapshot(7, &mut snapshots, &(), &mut ());

        assert!(snapshots.get(&7).unwrap().events.0.is_empty());
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
                    let mut context = Vec::new();
                    update_snapshot(7, &mut snapshots, &(), &mut context);
                    black_box((snapshots, context));
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
