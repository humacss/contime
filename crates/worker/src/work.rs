use std::time::{Duration, Instant};

use ahash::AHashMap;
use crossbeam_channel::{Receiver, RecvTimeoutError};

use crate::checkpoints::update_snapshot;
use crate::events::insert_batch;
use crate::listen::NotificationCollections;
use crate::query::{query_events, query_snapshots};
use crate::schedule::Schedule;
use crate::types::{
    AdvanceInput, ApplyInput, Checkpoints, Completion, EventQueryInput, EventQueryResponse, Events, QueryCheckpoints, QueryEvents,
    ReplayUpdate, RouteInput, SnapshotListenInput, SnapshotQueryInput, SnapshotQueryResponse, SnapshotSlot, WorkInput, WorkInputKind,
    WorkerConfig,
};

/// Receives event batches and schedules their checkpoint updates.
///
/// The caller chooses the execution context. This function does not create or
/// own a thread.
pub fn work<B, S, K>(
    input: Receiver<B>,
    config: WorkerConfig,
    events_config: S::Config,
    checkpoints_config: K::Config,
    mut checkpoints_context: <K as Checkpoints<S>>::Context,
) where
    B: ApplyInput,
    S: Events<<B::Route as RouteInput>::Input>,
    K: Checkpoints<S, Time = S::Time>,
    B::Completion: Completion<S::Rejection>,
{
    let mut snapshots = AHashMap::<u128, SnapshotSlot<S, K, B::Completion, S::Rejection>>::new();
    let mut schedule = Schedule::new(config.deadline_compaction_minimum, config.deadline_compaction_multiplier);
    let horizon = S::Time::default();

    loop {
        if schedule.is_empty() {
            match input.recv() {
                Ok(batch) => {
                    insert_batch(batch, &mut snapshots, &mut schedule, &events_config, &horizon, Instant::now());
                    update_budget::<S, K, B::Completion, S::Rejection, _>(
                        &mut snapshots,
                        &mut schedule,
                        &checkpoints_config,
                        &mut checkpoints_context,
                        config.replays_per_receive,
                        config.maximum_dirty_age,
                        &mut |_, _| {},
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
                insert_batch(batch, &mut snapshots, &mut schedule, &events_config, &horizon, Instant::now());
                update_budget::<S, K, B::Completion, S::Rejection, _>(
                    &mut snapshots,
                    &mut schedule,
                    &checkpoints_config,
                    &mut checkpoints_context,
                    config.replays_per_receive,
                    config.maximum_dirty_age,
                    &mut |_, _| {},
                );
            }
            Err(RecvTimeoutError::Timeout) => {
                update_overdue::<S, K, B::Completion, S::Rejection, _>(
                    &mut snapshots,
                    &mut schedule,
                    &checkpoints_config,
                    &mut checkpoints_context,
                    config.maximum_dirty_age,
                    &mut |_, _| {},
                );
            }
            Err(RecvTimeoutError::Disconnected) => {
                update_all::<S, K, B::Completion, S::Rejection, _>(
                    &mut snapshots,
                    &mut schedule,
                    &checkpoints_config,
                    &mut checkpoints_context,
                    &mut |_, _| {},
                );
                break;
            }
        }
    }
}

/// Receives apply and query messages on one worker-local queue.
pub fn work_messages<M, S, K>(
    input: Receiver<M>,
    config: WorkerConfig,
    events_config: S::Config,
    history_retention: <S as Events<<<M::Apply as ApplyInput>::Route as RouteInput>::Input>>::Time,
    checkpoints_config: K::Config,
    mut checkpoints_context: <K as Checkpoints<S>>::Context,
) where
    M: WorkInput,
    M::Apply: ApplyInput,
    S: Events<<<M::Apply as ApplyInput>::Route as RouteInput>::Input>
        + QueryEvents<
            <<M::Apply as ApplyInput>::Route as RouteInput>::Input,
            Time = <S as Events<<<M::Apply as ApplyInput>::Route as RouteInput>::Input>>::Time,
        >,
    K: Checkpoints<S, Time = <S as Events<<<M::Apply as ApplyInput>::Route as RouteInput>::Input>>::Time>
        + QueryCheckpoints<S, Context = <K as Checkpoints<S>>::Context>,
    <M::Apply as ApplyInput>::Completion: Completion<S::Rejection>,
    M::SnapshotQuery: SnapshotQueryInput<Time = <K as QueryCheckpoints<S>>::Time>,
    <M::SnapshotQuery as SnapshotQueryInput>::Response: SnapshotQueryResponse<<K as QueryCheckpoints<S>>::Snapshot>,
    M::EventQuery: EventQueryInput<Time = <S as QueryEvents<<<M::Apply as ApplyInput>::Route as RouteInput>::Input>>::Time>,
    <M::EventQuery as EventQueryInput>::Response: EventQueryResponse<<<M::Apply as ApplyInput>::Route as RouteInput>::Input>,
    M::SnapshotListen: SnapshotListenInput<Time = <S as Events<<<M::Apply as ApplyInput>::Route as RouteInput>::Input>>::Time>,
    M::Advance: AdvanceInput<Time = <S as Events<<<M::Apply as ApplyInput>::Route as RouteInput>::Input>>::Time>,
    <<M::Apply as ApplyInput>::Route as RouteInput>::Input: Clone,
{
    type ApplyRoute<M> = <<M as WorkInput>::Apply as ApplyInput>::Route;
    type ApplyCompletion<M> = <<M as WorkInput>::Apply as ApplyInput>::Completion;
    type ApplyEvent<M> = <ApplyRoute<M> as RouteInput>::Input;
    type EventTime<M, S> = <S as Events<ApplyEvent<M>>>::Time;

    let mut snapshots = AHashMap::<u128, SnapshotSlot<S, K, ApplyCompletion<M>, S::Rejection>>::new();
    let mut listeners = NotificationCollections::<EventTime<M, S>, <M::SnapshotListen as SnapshotListenInput>::Listener>::new();
    let mut schedule = Schedule::new(config.deadline_compaction_minimum, config.deadline_compaction_multiplier);
    let mut current_time = EventTime::<M, S>::default();
    let mut horizon = EventTime::<M, S>::default();

    loop {
        let received = if schedule.is_empty() {
            input.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            let deadline = schedule.next_deadline(config.maximum_dirty_age).expect("dirty schedule had no deadline");
            input.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        };

        match received {
            Ok(message) => match message.into_kind() {
                WorkInputKind::Apply(batch) => {
                    insert_batch(batch, &mut snapshots, &mut schedule, &events_config, &horizon, Instant::now());
                    update_budget::<S, K, ApplyCompletion<M>, S::Rejection, _>(
                        &mut snapshots,
                        &mut schedule,
                        &checkpoints_config,
                        &mut checkpoints_context,
                        config.replays_per_receive,
                        config.maximum_dirty_age,
                        &mut |update, snapshots| listeners.record(update, snapshots),
                    );
                    listeners.flush();
                }
                WorkInputKind::SnapshotQuery(query) => query_snapshots(query, &snapshots, &checkpoints_config, &mut checkpoints_context),
                WorkInputKind::EventQuery(query) => query_events::<_, ApplyEvent<M>, _, _, _, _>(query, &snapshots),
                WorkInputKind::SnapshotListen(registration) => {
                    let (time, snapshot_ids, listener) = registration.into_parts();
                    listeners.register(time, snapshot_ids, listener, &mut snapshots);
                }
                WorkInputKind::Advance(advance) => {
                    let (target_time, completion) = advance.into_parts();
                    crate::advance::advance_worker::<ApplyEvent<M>, _, _, _, _, _>(
                        &mut snapshots,
                        &mut schedule,
                        &checkpoints_config,
                        &mut checkpoints_context,
                        &mut current_time,
                        &mut horizon,
                        &history_retention,
                        target_time,
                        completion,
                        &mut |update, snapshots| listeners.record(update, snapshots),
                    );
                    listeners.flush();
                }
            },
            Err(RecvTimeoutError::Timeout) => {
                update_overdue::<S, K, ApplyCompletion<M>, S::Rejection, _>(
                    &mut snapshots,
                    &mut schedule,
                    &checkpoints_config,
                    &mut checkpoints_context,
                    config.maximum_dirty_age,
                    &mut |update, snapshots| listeners.record(update, snapshots),
                );
                listeners.flush();
            }
            Err(RecvTimeoutError::Disconnected) => {
                update_all::<S, K, ApplyCompletion<M>, S::Rejection, _>(
                    &mut snapshots,
                    &mut schedule,
                    &checkpoints_config,
                    &mut checkpoints_context,
                    &mut |update, snapshots| listeners.record(update, snapshots),
                );
                listeners.flush();
                break;
            }
        }
    }
}

fn update_budget<S, K, C, R, F>(
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    schedule: &mut Schedule,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
    replay_budget: usize,
    maximum_dirty_age: Duration,
    on_replayed: &mut F,
) where
    K: Checkpoints<S>,
    C: Completion<R>,
    F: FnMut(ReplayUpdate<K::Time>, &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>),
{
    for _ in 0..replay_budget {
        let Some(snapshot_id) = schedule.pop_next(Instant::now(), maximum_dirty_age) else {
            break;
        };
        let update = update_snapshot(snapshot_id, snapshots, checkpoints_config, checkpoints_context);
        on_replayed(update, snapshots);
    }
}

fn update_overdue<S, K, C, R, F>(
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    schedule: &mut Schedule,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
    maximum_dirty_age: Duration,
    on_replayed: &mut F,
) where
    K: Checkpoints<S>,
    C: Completion<R>,
    F: FnMut(ReplayUpdate<K::Time>, &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>),
{
    loop {
        let Some(snapshot_id) = schedule.pop_overdue(Instant::now(), maximum_dirty_age) else {
            break;
        };
        let update = update_snapshot(snapshot_id, snapshots, checkpoints_config, checkpoints_context);
        on_replayed(update, snapshots);
    }
}

fn update_all<S, K, C, R, F>(
    snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    schedule: &mut Schedule,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut K::Context,
    on_replayed: &mut F,
) where
    K: Checkpoints<S>,
    C: Completion<R>,
    F: FnMut(ReplayUpdate<K::Time>, &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>),
{
    while let Some(snapshot_id) = schedule.pop_largest(Instant::now()) {
        let update = update_snapshot(snapshot_id, snapshots, checkpoints_config, checkpoints_context);
        on_replayed(update, snapshots);
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
    use crate::{ApplyBatch, Checkpoints, EventInsert, Events, RoutedInput, WorkerConfig};

    struct TestInput(u128);

    #[derive(Default)]
    struct TestEvents(Vec<u128>);

    impl Events<TestInput> for TestEvents {
        type Config = ();
        type Rejection = ();
        type Time = u64;

        fn create(_id: u128, _config: &(), _horizon: &u64) -> Self {
            Self::default()
        }

        fn insert(&mut self, input: TestInput) -> EventInsert<()> {
            self.0.push(input.0);
            EventInsert { changed: true, rejections: Vec::new() }
        }

        fn dirty_time(&self) -> &u64 {
            &0
        }

        fn prune_before(&mut self, _horizon: &u64) {}
    }

    struct TestCheckpoints;

    impl Checkpoints<TestEvents> for TestCheckpoints {
        type Config = ();
        type Context = Arc<Mutex<Vec<Vec<u128>>>>;
        type Time = u64;

        fn create(_id: u128, _config: &()) -> Self {
            Self
        }

        fn update(&mut self, events: &mut TestEvents, context: &mut Self::Context) -> Self::Time {
            context.lock().unwrap().push(events.0.clone());
            0
        }

        fn advance_before(&mut self, _events: &TestEvents, _context: &mut Self::Context, _horizon: &u64) {}
    }

    type TestCompletion = crossbeam_channel::Sender<Vec<()>>;

    fn batch(first_id: u128, count: u128) -> ApplyBatch<TestInput, TestCompletion> {
        let (completion, _responses) = unbounded();
        let inputs = (0..count).map(|offset| RoutedInput { snapshot_id: 7, input: TestInput(first_id + offset) }).collect();
        ApplyBatch { inputs, completion }
    }

    fn config(replays_per_receive: usize) -> WorkerConfig {
        WorkerConfig {
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

        work::<_, TestEvents, TestCheckpoints>(receiver, config(1), (), (), Arc::clone(&context));

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
                    work::<_, TestEvents, TestCheckpoints>(receiver, config(1), (), (), Arc::clone(&context));
                    black_box(context);
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
