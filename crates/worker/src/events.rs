use std::collections::hash_map::Entry;
use std::time::Instant;

use ahash::AHashMap;

use crate::memory::Memory;
use crate::schedule::Schedule;
use crate::types::{
    finish_if_ready, new_request, register_waiter, ApplyBatch, Completion, Events, Request, RoutedInput, SnapshotSlot, WorkerInput,
    WorkerRejection,
};

pub(crate) fn insert_batch<E, S, K, C>(
    batch: ApplyBatch<E, C>,
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, S::Rejection>>,
    schedule: &mut Schedule,
    memory: &mut Memory,
    events_config: &S::Config,
    now: Instant,
) where
    E: WorkerInput,
    S: Events<E>,
    C: Completion<S::Rejection>,
{
    let reserved_bytes = batch.inputs.iter().fold(0_u64, |total, routed| total.saturating_add(routed.input.conservative_size()));

    if !memory.try_reserve(reserved_bytes) {
        batch.completion.reject(memory_full_rejections(&batch.inputs));
        return;
    }

    let request = new_request(batch.completion);
    for routed in batch.inputs {
        insert_event(routed, &request, snapshots, schedule, memory, events_config, now);
    }
    finish_if_ready(&request);
}

fn insert_event<E, S, K, C>(
    routed: RoutedInput<E>,
    request: &Request<C, S::Rejection>,
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, S::Rejection>>,
    schedule: &mut Schedule,
    memory: &mut Memory,
    events_config: &S::Config,
    now: Instant,
) where
    E: WorkerInput,
    S: Events<E>,
    C: Completion<S::Rejection>,
{
    let input_bytes = routed.input.conservative_size();
    let retained_limit = memory.retained_limit_for(input_bytes);
    let mut creation_delta = 0_i64;

    let slot = match snapshots.entry(routed.snapshot_id) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            let Some(created) = S::create(routed.snapshot_id, events_config, retained_limit) else {
                request.borrow_mut().rejections.push(WorkerRejection::MemoryFull { input_id: routed.input.input_id() });
                memory.reconcile(input_bytes, 0);
                return;
            };
            creation_delta = created.retained_bytes_delta;
            entry.insert(SnapshotSlot { events: created.events, checkpoints: None, waiters: Vec::new() })
        }
    };

    let result = slot.events.insert(routed.input, limit_after_delta(retained_limit, creation_delta));
    memory.reconcile(input_bytes, creation_delta.saturating_add(result.retained_bytes_delta));
    request.borrow_mut().rejections.extend(result.rejections.into_iter().map(WorkerRejection::Event));

    if result.changed {
        schedule.mark_dirty(routed.snapshot_id, now);
        register_waiter(slot, request);
    }
}

fn limit_after_delta(limit: u64, delta: i64) -> u64 {
    if delta >= 0 {
        limit.saturating_sub(delta as u64)
    } else {
        limit.saturating_add(delta.unsigned_abs())
    }
}

fn memory_full_rejections<E, R>(inputs: &[RoutedInput<E>]) -> Vec<WorkerRejection<R>>
where
    E: WorkerInput,
{
    let mut input_ids = inputs.iter().map(|routed| routed.input.input_id()).collect::<Vec<_>>();
    input_ids.sort_unstable();
    input_ids.dedup();
    input_ids.into_iter().map(|input_id| WorkerRejection::MemoryFull { input_id }).collect()
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ahash::AHashMap;
    use criterion::Criterion;
    use crossbeam_channel::{unbounded, TryRecvError};

    use super::insert_batch;
    use crate::memory::Memory;
    use crate::schedule::Schedule;
    use crate::types::SnapshotSlot;
    use crate::{ApplyBatch, EventInsert, Events, EventsCreated, RoutedInput, WorkerInput, WorkerRejection};

    struct TestInput(u128);

    impl WorkerInput for TestInput {
        fn input_id(&self) -> u128 {
            self.0
        }
        fn conservative_size(&self) -> u64 {
            32
        }
    }

    #[derive(Default)]
    struct TestEvents(Vec<u128>);

    impl Events<TestInput> for TestEvents {
        type Config = ();
        type Rejection = ();

        fn create(_id: u128, _config: &(), _limit: u64) -> Option<EventsCreated<Self>> {
            Some(EventsCreated { events: Self::default(), retained_bytes_delta: 0 })
        }

        fn insert(&mut self, input: Arc<TestInput>, _limit: u64) -> EventInsert<()> {
            self.0.push(input.0);
            EventInsert { retained_bytes_delta: 32, changed: true, rejections: Vec::new() }
        }
    }

    fn batch(
        count: u128,
    ) -> (ApplyBatch<TestInput, crossbeam_channel::Sender<Vec<WorkerRejection<()>>>>, crossbeam_channel::Receiver<Vec<WorkerRejection<()>>>)
    {
        let (completion, responses) = unbounded();
        let inputs = (0..count).map(|id| RoutedInput { snapshot_id: 7, input: Arc::new(TestInput(id)) }).collect();
        (ApplyBatch { inputs, completion }, responses)
    }

    #[test]
    fn events_are_inserted_without_completing_before_checkpoints_update() {
        let (batch, responses) = batch(2);
        let mut snapshots = AHashMap::new();
        let mut schedule = Schedule::new(usize::MAX, 2);
        let mut memory = Memory::new(1_000);

        insert_batch::<_, TestEvents, (), _>(batch, &mut snapshots, &mut schedule, &mut memory, &(), Instant::now());

        assert_eq!(snapshots.get(&7).unwrap().events.0, vec![0, 1]);
        assert!(!schedule.is_empty());
        assert_eq!(responses.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_events() {
        let mut criterion = Criterion::default();
        criterion.bench_function("worker/events/1000_inputs/one_snapshot", |bencher| {
            let mut snapshots = AHashMap::new();
            snapshots.insert(
                7,
                SnapshotSlot::<TestEvents, (), _, ()> {
                    events: TestEvents(Vec::with_capacity(1_000)),
                    checkpoints: None,
                    waiters: Vec::new(),
                },
            );
            let mut schedule = Schedule::new(usize::MAX, 2);
            let mut memory = Memory::new(1_000_000);

            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let batch = batch(1_000).0;
                    let started = Instant::now();
                    insert_batch::<_, TestEvents, (), _>(batch, &mut snapshots, &mut schedule, &mut memory, &(), Instant::now());
                    measured += started.elapsed();

                    let slot = snapshots.get_mut(&7).unwrap();
                    slot.events.0.clear();
                    slot.waiters.clear();
                    while schedule.pop_largest(Instant::now()).is_some() {}
                    schedule.is_empty();
                    memory.apply_delta(-(memory.used() as i64));
                }
                black_box((&snapshots, &schedule, &memory));
                measured
            });
        });
        criterion.final_summary();
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_event_components() {
        let mut criterion = Criterion::default();
        let inputs = (0..1_000_u128).map(|id| Arc::new(TestInput(id))).collect::<Vec<_>>();

        criterion.bench_function("worker/events/components/1000_reservation_scan", |bencher| {
            bencher.iter(|| {
                let bytes = inputs.iter().fold(0_u64, |total, input| total.saturating_add(input.conservative_size()));
                black_box(bytes);
            });
        });

        criterion.bench_function("worker/events/components/1000_occupied_snapshot_lookups", |bencher| {
            let mut snapshots = AHashMap::new();
            snapshots.insert(
                7,
                SnapshotSlot::<TestEvents, (), crossbeam_channel::Sender<Vec<WorkerRejection<()>>>, ()> {
                    events: TestEvents(Vec::with_capacity(1_000)),
                    checkpoints: None,
                    waiters: Vec::new(),
                },
            );
            bencher.iter(|| {
                for _ in 0..1_000 {
                    black_box(snapshots.get_mut(&black_box(7)).unwrap());
                }
            });
        });

        criterion.bench_function("worker/events/components/1000_event_store_inserts", |bencher| {
            let mut events = TestEvents(Vec::with_capacity(1_000));
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let inputs = inputs.iter().cloned().collect::<Vec<_>>();
                    let started = Instant::now();
                    for input in inputs {
                        black_box(events.insert(input, u64::MAX));
                    }
                    measured += started.elapsed();
                    events.0.clear();
                }
                measured
            });
        });

        criterion.bench_function("worker/events/components/1000_existing_waiter_checks", |bencher| {
            let (completion, _responses) = unbounded::<Vec<WorkerRejection<()>>>();
            let request = crate::types::new_request(completion);
            let mut slot = SnapshotSlot::<TestEvents, (), _, ()> { events: TestEvents(Vec::new()), checkpoints: None, waiters: Vec::new() };
            crate::types::register_waiter(&mut slot, &request);
            bencher.iter(|| {
                for _ in 0..1_000 {
                    crate::types::register_waiter(&mut slot, &request);
                }
            });
        });

        criterion.bench_function("worker/events/components/1000_empty_rejection_extensions", |bencher| {
            let (completion, _responses) = unbounded::<Vec<WorkerRejection<()>>>();
            let request = crate::types::new_request::<_, ()>(completion);
            bencher.iter(|| {
                for _ in 0..1_000 {
                    request.borrow_mut().rejections.extend(std::iter::empty());
                }
            });
        });

        criterion.final_summary();
    }
}
