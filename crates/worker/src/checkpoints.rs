use ahash::AHashMap;

use crate::types::{complete_snapshot, Checkpoints, Completion, ReplayUpdate, SnapshotSlot};

pub(crate) fn update_snapshot<S, K, C, R>(
    snapshot_id: u128,
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
) -> ReplayUpdate<K::Time>
where
    K: Checkpoints<S>,
    C: Completion<R>,
{
    let slot = snapshots.get_mut(&snapshot_id).expect("dirty schedule referenced a missing event store");
    let events = slot.events.as_mut().expect("dirty schedule referenced a metadata-only snapshot slot");

    if slot.checkpoints.is_none() {
        slot.checkpoints = Some(K::create(snapshot_id, checkpoints_config));
    }

    let checkpoints = slot.checkpoints.as_mut().expect("checkpoint store was not initialized");
    let affected_from = checkpoints.update(events, checkpoints_context);

    for request in slot.waiters.drain(..) {
        complete_snapshot(request);
    }

    ReplayUpdate { snapshot_id, affected_from }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use ahash::AHashMap;
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, TryRecvError};

    use super::update_snapshot;
    use crate::types::{new_request, register_waiter, ReplayUpdate, SnapshotSlot};
    use crate::Checkpoints;

    struct TestEvents(Vec<u128>);

    struct AcknowledgingCheckpoints;

    impl Checkpoints<TestEvents> for AcknowledgingCheckpoints {
        type Config = ();
        type Context = ();
        type Time = u64;

        fn create(_snapshot_id: u128, _config: &()) -> Self {
            Self
        }

        fn update(&mut self, events: &mut TestEvents, _context: &mut ()) -> Self::Time {
            events.0.clear();
            0
        }

        fn advance_before(&mut self, _events: &TestEvents, _context: &mut (), _horizon: &u64) {}
    }

    #[derive(Default)]
    struct TestCheckpoints {
        event_count: usize,
    }

    impl Checkpoints<TestEvents> for TestCheckpoints {
        type Config = ();
        type Context = Vec<usize>;
        type Time = u64;

        fn create(_snapshot_id: u128, _config: &()) -> Self {
            Self::default()
        }

        fn update(&mut self, events: &mut TestEvents, context: &mut Vec<usize>) -> Self::Time {
            self.event_count = events.0.len();
            context.push(self.event_count);
            37
        }

        fn advance_before(&mut self, _events: &TestEvents, _context: &mut Vec<usize>, _horizon: &u64) {}
    }

    fn snapshot() -> (
        AHashMap<u128, SnapshotSlot<TestEvents, TestCheckpoints, crossbeam_channel::Sender<Vec<()>>, ()>>,
        crossbeam_channel::Receiver<Vec<()>>,
    ) {
        let (completion, responses) = unbounded::<Vec<()>>();
        let request = new_request(completion);
        let mut slot = SnapshotSlot::with_events(TestEvents((0..1_000).collect()));
        register_waiter(&mut slot, &request);
        let mut snapshots = AHashMap::new();
        snapshots.insert(7, slot);
        (snapshots, responses)
    }

    #[test]
    fn checkpoint_update_returns_the_affected_interval_start() {
        let (mut snapshots, responses) = snapshot();
        let mut context = Vec::new();

        let update = update_snapshot(7, &mut snapshots, &(), &mut context);

        assert_eq!(update, ReplayUpdate { snapshot_id: 7, affected_from: 37 });
        assert_eq!(context, vec![1_000]);
        assert_eq!(snapshots.get(&7).unwrap().checkpoints.as_ref().unwrap().event_count, 1_000);
        assert_eq!(responses.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn checkpoint_update_can_acknowledge_mutable_event_history() {
        let (completion, responses) = unbounded::<Vec<()>>();
        let request = new_request(completion);
        let mut slot = SnapshotSlot::<_, AcknowledgingCheckpoints, _, _>::with_events(TestEvents(vec![1, 2, 3]));
        register_waiter(&mut slot, &request);
        let mut snapshots = AHashMap::new();
        snapshots.insert(7, slot);

        update_snapshot(7, &mut snapshots, &(), &mut ());

        assert!(snapshots.get(&7).unwrap().events.as_ref().unwrap().0.is_empty());
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
