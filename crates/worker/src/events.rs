use std::collections::hash_map::Entry;
use std::time::Instant;

use ahash::AHashMap;

use crate::schedule::Schedule;
use crate::types::{finish_if_ready, new_request, register_waiter, ApplyInput, Completion, Events, Request, RouteInput, SnapshotSlot};

pub(crate) fn insert_batch<B, S, K>(
    batch: B,
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, B::Completion, S::Rejection>>,
    schedule: &mut Schedule,
    events_config: &S::Config,
    now: Instant,
) where
    B: ApplyInput,
    S: Events<<B::Route as RouteInput>::Input>,
    B::Completion: Completion<S::Rejection>,
{
    let (inputs, completion) = batch.into_parts();
    let request = new_request(completion);
    for routed in inputs {
        insert_event(routed, &request, snapshots, schedule, events_config, now);
    }
    finish_if_ready(&request);
}

fn insert_event<R, S, K, C>(
    routed: R,
    request: &Request<C, S::Rejection>,
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, S::Rejection>>,
    schedule: &mut Schedule,
    events_config: &S::Config,
    now: Instant,
) where
    R: RouteInput,
    S: Events<R::Input>,
    C: Completion<S::Rejection>,
{
    let (snapshot_id, input) = routed.into_parts();
    let slot = match snapshots.entry(snapshot_id) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            entry.insert(SnapshotSlot { events: S::create(snapshot_id, events_config), checkpoints: None, waiters: Vec::new() })
        }
    };

    let result = slot.events.insert(input);
    request.borrow_mut().rejections.extend(result.rejections);

    if result.changed {
        schedule.mark_dirty(snapshot_id, now);
        register_waiter(slot, request);
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    use ahash::AHashMap;
    use criterion::Criterion;
    use crossbeam_channel::{unbounded, TryRecvError};

    use super::insert_batch;
    use crate::schedule::Schedule;
    use crate::types::SnapshotSlot;
    use crate::{ApplyBatch, ApplyInput, EventInsert, Events, RouteInput, RoutedInput};

    #[derive(Clone)]
    struct TestInput(u128);

    #[derive(Default)]
    struct TestEvents(Vec<u128>);

    impl Events<TestInput> for TestEvents {
        type Config = ();
        type Rejection = ();

        fn create(_id: u128, _config: &()) -> Self {
            Self::default()
        }

        fn insert(&mut self, input: TestInput) -> EventInsert<()> {
            self.0.push(input.0);
            EventInsert { changed: true, rejections: Vec::new() }
        }
    }

    #[derive(Default)]
    struct DirectEvents(Vec<u128>);

    impl Events<TestInput> for DirectEvents {
        type Config = ();
        type Rejection = ();

        fn create(_id: u128, _config: &()) -> Self {
            Self::default()
        }

        fn insert(&mut self, input: TestInput) -> EventInsert<()> {
            self.0.push(input.0);
            EventInsert { changed: true, rejections: Vec::new() }
        }
    }

    struct AdapterRoute {
        snapshot_id: u128,
        input: TestInput,
    }

    impl RouteInput for AdapterRoute {
        type Input = TestInput;

        fn into_parts(self) -> (u128, Self::Input) {
            (self.snapshot_id, self.input)
        }
    }

    struct AdapterBatch<C> {
        inputs: Vec<AdapterRoute>,
        completion: C,
    }

    impl<C> ApplyInput for AdapterBatch<C> {
        type Route = AdapterRoute;
        type Completion = C;

        fn into_parts(self) -> (Vec<Self::Route>, Self::Completion) {
            (self.inputs, self.completion)
        }
    }

    #[test]
    fn worker_consumes_caller_selected_batch_and_route_types() {
        let (completion, _responses) = unbounded();
        let batch = AdapterBatch { inputs: vec![AdapterRoute { snapshot_id: 7, input: TestInput(9) }], completion };
        let mut snapshots = AHashMap::new();
        let mut schedule = Schedule::new(usize::MAX, 2);

        insert_batch::<_, DirectEvents, ()>(batch, &mut snapshots, &mut schedule, &(), Instant::now());

        assert_eq!(snapshots.get(&7).unwrap().events.0, vec![9]);
    }

    #[test]
    fn worker_preserves_the_selected_input_ownership_type() {
        let (completion, _responses) = unbounded();
        let batch = ApplyBatch { inputs: vec![RoutedInput { snapshot_id: 7, input: TestInput(9) }], completion };
        let mut snapshots = AHashMap::new();
        let mut schedule = Schedule::new(usize::MAX, 2);

        insert_batch::<_, DirectEvents, ()>(batch, &mut snapshots, &mut schedule, &(), Instant::now());

        assert_eq!(snapshots.get(&7).unwrap().events.0, vec![9]);
    }

    fn batch(count: u128) -> (ApplyBatch<TestInput, crossbeam_channel::Sender<Vec<()>>>, crossbeam_channel::Receiver<Vec<()>>) {
        let (completion, responses) = unbounded();
        let inputs = (0..count).map(|id| RoutedInput { snapshot_id: 7, input: TestInput(id) }).collect();
        (ApplyBatch { inputs, completion }, responses)
    }

    #[test]
    fn events_are_inserted_without_completing_before_checkpoints_update() {
        let (batch, responses) = batch(2);
        let mut snapshots = AHashMap::new();
        let mut schedule = Schedule::new(usize::MAX, 2);

        insert_batch::<_, TestEvents, ()>(batch, &mut snapshots, &mut schedule, &(), Instant::now());

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

            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let batch = batch(1_000).0;
                    let started = Instant::now();
                    insert_batch::<_, TestEvents, ()>(batch, &mut snapshots, &mut schedule, &(), Instant::now());
                    measured += started.elapsed();

                    let slot = snapshots.get_mut(&7).unwrap();
                    slot.events.0.clear();
                    slot.waiters.clear();
                    while schedule.pop_largest(Instant::now()).is_some() {}
                    schedule.is_empty();
                }
                black_box((&snapshots, &schedule));
                measured
            });
        });
        criterion.final_summary();
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_event_components() {
        let mut criterion = Criterion::default();
        let inputs = (0..1_000_u128).map(TestInput).collect::<Vec<_>>();

        criterion.bench_function("worker/events/components/1000_occupied_snapshot_lookups", |bencher| {
            let mut snapshots = AHashMap::new();
            snapshots.insert(
                7,
                SnapshotSlot::<TestEvents, (), crossbeam_channel::Sender<Vec<()>>, ()> {
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
                        black_box(events.insert(input));
                    }
                    measured += started.elapsed();
                    events.0.clear();
                }
                measured
            });
        });

        criterion.bench_function("worker/events/components/1000_existing_waiter_checks", |bencher| {
            let (completion, _responses) = unbounded::<Vec<()>>();
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
            let (completion, _responses) = unbounded::<Vec<()>>();
            let request = crate::types::new_request::<_, ()>(completion);
            bencher.iter(|| {
                for _ in 0..1_000 {
                    request.borrow_mut().rejections.extend(std::iter::empty::<()>());
                }
            });
        });

        criterion.final_summary();
    }
}
