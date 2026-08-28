use std::time::{Duration, Instant};

use ahash::AHashMap;
use crossbeam_channel::{Receiver, RecvTimeoutError};

use crate::checkpoints::update_snapshot;
use crate::events::insert_batch;
use crate::memory::Memory;
use crate::schedule::Schedule;
use crate::types::{ApplyBatch, Checkpoints, Completion, Events, SnapshotSlot, WorkerConfig, WorkerInput};

/// Receives event batches and schedules their checkpoint updates.
///
/// The caller chooses the execution context. This function does not create or
/// own a thread.
pub fn work<E, S, K, C>(
    input: Receiver<ApplyBatch<E, C>>,
    config: WorkerConfig,
    events_config: S::Config,
    checkpoints_config: K::Config,
    mut checkpoints_context: K::Context,
) where
    E: WorkerInput,
    S: Events<E>,
    K: Checkpoints<S>,
    C: Completion<S::Rejection>,
{
    let mut snapshots = AHashMap::<u128, SnapshotSlot<S, K, C, S::Rejection>>::new();
    let mut schedule = Schedule::new(config.deadline_compaction_minimum, config.deadline_compaction_multiplier);
    let mut memory = Memory::new(config.memory_limit);

    loop {
        if schedule.is_empty() {
            match input.recv() {
                Ok(batch) => {
                    insert_batch(batch, &mut snapshots, &mut schedule, &mut memory, &events_config, Instant::now());
                    update_budget::<S, K, C, S::Rejection>(
                        &mut snapshots,
                        &mut schedule,
                        &mut memory,
                        &checkpoints_config,
                        &mut checkpoints_context,
                        config.replays_per_receive,
                        config.maximum_dirty_age,
                    );
                }
                Err(_) => break,
            }
            continue;
        }

        let deadline = schedule.next_deadline(config.maximum_dirty_age).expect("dirty schedule had no deadline");
        let timeout = deadline.saturating_duration_since(Instant::now());

        match input.recv_timeout(timeout) {
            Ok(batch) => {
                insert_batch(batch, &mut snapshots, &mut schedule, &mut memory, &events_config, Instant::now());
                update_budget::<S, K, C, S::Rejection>(
                    &mut snapshots,
                    &mut schedule,
                    &mut memory,
                    &checkpoints_config,
                    &mut checkpoints_context,
                    config.replays_per_receive,
                    config.maximum_dirty_age,
                );
            }
            Err(RecvTimeoutError::Timeout) => {
                update_overdue::<S, K, C, S::Rejection>(
                    &mut snapshots,
                    &mut schedule,
                    &mut memory,
                    &checkpoints_config,
                    &mut checkpoints_context,
                    config.maximum_dirty_age,
                );
            }
            Err(RecvTimeoutError::Disconnected) => {
                update_all::<S, K, C, S::Rejection>(
                    &mut snapshots,
                    &mut schedule,
                    &mut memory,
                    &checkpoints_config,
                    &mut checkpoints_context,
                );
                break;
            }
        }
    }
}

fn update_budget<S, K, C, R>(
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    schedule: &mut Schedule,
    memory: &mut Memory,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
    replay_budget: usize,
    maximum_dirty_age: Duration,
) where
    K: Checkpoints<S>,
    C: Completion<R>,
{
    for _ in 0..replay_budget {
        let Some(snapshot_id) = schedule.pop_next(Instant::now(), maximum_dirty_age) else {
            break;
        };
        update_snapshot(snapshot_id, snapshots, memory, checkpoints_config, checkpoints_context);
    }
}

fn update_overdue<S, K, C, R>(
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    schedule: &mut Schedule,
    memory: &mut Memory,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
    maximum_dirty_age: Duration,
) where
    K: Checkpoints<S>,
    C: Completion<R>,
{
    loop {
        let Some(snapshot_id) = schedule.pop_overdue(Instant::now(), maximum_dirty_age) else {
            break;
        };
        update_snapshot(snapshot_id, snapshots, memory, checkpoints_config, checkpoints_context);
    }
}

fn update_all<S, K, C, R>(
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    schedule: &mut Schedule,
    memory: &mut Memory,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
) where
    K: Checkpoints<S>,
    C: Completion<R>,
{
    while let Some(snapshot_id) = schedule.pop_largest(Instant::now()) {
        update_snapshot(snapshot_id, snapshots, memory, checkpoints_config, checkpoints_context);
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::unbounded;

    use super::work;
    use crate::{
        ApplyBatch, CheckpointResult, Checkpoints, CheckpointsCreated, EventInsert, Events, EventsCreated, RoutedInput, WorkerConfig,
        WorkerInput, WorkerRejection,
    };

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

    struct TestCheckpoints;

    impl Checkpoints<TestEvents> for TestCheckpoints {
        type Config = ();
        type Context = Arc<Mutex<Vec<Vec<u128>>>>;

        fn create(_id: u128, _config: &(), _limit: u64) -> CheckpointsCreated<Self> {
            CheckpointsCreated { checkpoints: Self, retained_bytes_delta: 0 }
        }

        fn update(&mut self, events: &TestEvents, context: &mut Self::Context, _limit: u64) -> CheckpointResult {
            context.lock().unwrap().push(events.0.clone());
            CheckpointResult { retained_bytes_delta: 0 }
        }
    }

    type TestCompletion = crossbeam_channel::Sender<Vec<WorkerRejection<()>>>;

    fn batch(first_id: u128, count: u128) -> ApplyBatch<TestInput, TestCompletion> {
        let (completion, _responses) = unbounded();
        let inputs = (0..count).map(|offset| RoutedInput { snapshot_id: 7, input: Arc::new(TestInput(first_id + offset)) }).collect();
        ApplyBatch { inputs, completion }
    }

    fn config(replays_per_receive: usize) -> WorkerConfig {
        WorkerConfig {
            memory_limit: 10_000_000,
            maximum_dirty_age: Duration::from_micros(100),
            replays_per_receive,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        }
    }

    #[test]
    fn one_batch_is_inserted_before_its_checkpoints_are_updated() {
        let (sender, receiver) = unbounded();
        let context = Arc::new(Mutex::new(Vec::new()));
        sender.send(batch(1, 3)).unwrap();
        drop(sender);

        work::<TestInput, TestEvents, TestCheckpoints, _>(receiver, config(1), (), (), Arc::clone(&context));

        assert_eq!(*context.lock().unwrap(), vec![vec![1, 2, 3]]);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_work() {
        let mut criterion = Criterion::default();
        criterion.bench_function("worker/work/100_batches/1000_inputs", |bencher| {
            bencher.iter_batched(
                || {
                    let (sender, receiver) = unbounded();
                    for batch_index in 0..100_u128 {
                        sender.send(batch(batch_index * 1_000, 1_000)).unwrap();
                    }
                    drop(sender);
                    (receiver, Arc::new(Mutex::new(Vec::new())))
                },
                |(receiver, context)| {
                    work::<TestInput, TestEvents, TestCheckpoints, _>(receiver, config(1), (), (), Arc::clone(&context));
                    black_box(context);
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
