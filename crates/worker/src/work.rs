use ahash::AHashMap;
use crossbeam_channel::{Receiver, TryRecvError};

use crate::checkpoints::update_snapshot;
use crate::events::insert_batch;
use crate::listen::NotificationCollections;
use crate::query::{query_events, query_snapshots};
use crate::types::{
    AdvanceInput, ApplyInput, Checkpoints, Completion, EventQueryInput, EventQueryResponse, Events, QueryCheckpoints, QueryEvents,
    RouteInput, SnapshotListenInput, SnapshotQueryInput, SnapshotQueryResponse, SnapshotSlot, WorkInput, WorkInputKind, WorkerConfig,
};

/// Drains ready event batches into one apply cycle and updates each changed
/// snapshot once per cycle.
///
/// The caller chooses the execution context. This function does not create or
/// own a thread.
pub fn work<B, S, K>(
    input: Receiver<B>,
    config: WorkerConfig,
    events_config: S::Config,
    checkpoints_config: K::Config,
    checkpoints_context: <K as Checkpoints<S>>::Context,
) where
    B: ApplyInput,
    S: Events<<B::Route as RouteInput>::Input>,
    K: Checkpoints<S, Time = S::Time>,
    B::Completion: Completion<S::Rejection>,
{
    work_with_batch_limit::<B, S, K>(input, config, events_config, checkpoints_config, checkpoints_context, usize::MAX);
}

fn work_with_batch_limit<B, S, K>(
    input: Receiver<B>,
    _config: WorkerConfig,
    events_config: S::Config,
    checkpoints_config: K::Config,
    mut checkpoints_context: <K as Checkpoints<S>>::Context,
    maximum_batches_per_cycle: usize,
) where
    B: ApplyInput,
    S: Events<<B::Route as RouteInput>::Input>,
    K: Checkpoints<S, Time = S::Time>,
    B::Completion: Completion<S::Rejection>,
{
    assert!(maximum_batches_per_cycle > 0);
    let mut snapshots = AHashMap::<u128, SnapshotSlot<S, K, B::Completion, S::Rejection>>::new();
    let mut dirty_snapshot_ids = Vec::new();
    let horizon = S::Time::default();

    while let Ok(batch) = input.recv() {
        dirty_snapshot_ids.clear();
        insert_batch(batch, &mut snapshots, &mut dirty_snapshot_ids, &events_config, &horizon);
        for batch in input.try_iter().take(maximum_batches_per_cycle - 1) {
            insert_batch(batch, &mut snapshots, &mut dirty_snapshot_ids, &events_config, &horizon);
        }
        for snapshot_id in dirty_snapshot_ids.drain(..) {
            update_snapshot(snapshot_id, &mut snapshots, &checkpoints_config, &mut checkpoints_context);
        }
    }
}

type ApplyRoute<M> = <<M as WorkInput>::Apply as ApplyInput>::Route;
type ApplyCompletion<M> = <<M as WorkInput>::Apply as ApplyInput>::Completion;
type ApplyEvent<M> = <ApplyRoute<M> as RouteInput>::Input;
type EventTime<M, S> = <S as Events<ApplyEvent<M>>>::Time;
type MessageKind<M> = WorkInputKind<
    <M as WorkInput>::Apply,
    <M as WorkInput>::SnapshotQuery,
    <M as WorkInput>::EventQuery,
    <M as WorkInput>::SnapshotListen,
    <M as WorkInput>::Advance,
>;

/// Runs with activity subscriptions: false means idle, true means working.
/// Registrations are serviced only in the idle phase; work receivers are ordinary channels.
pub fn work_messages<M, S, K>(
    input: Receiver<M>,
    _config: WorkerConfig,
    events_config: S::Config,
    history_retention: <S as Events<<<M::Apply as ApplyInput>::Route as RouteInput>::Input>>::Time,
    checkpoints_config: K::Config,
    checkpoints_context: <K as Checkpoints<S>>::Context,
    registrations: Receiver<crossbeam_channel::Sender<bool>>,
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
    let mut worker =
        MessageWorker::<M, S, K>::new(&input, &registrations, events_config, history_retention, checkpoints_config, checkpoints_context);
    worker.activity_loop();
}

struct MessageWorker<'a, M, S, K>
where
    M: WorkInput,
    M::Apply: ApplyInput,
    S: Events<ApplyEvent<M>>,
    K: Checkpoints<S, Time = EventTime<M, S>>,
    ApplyCompletion<M>: Completion<S::Rejection>,
    M::SnapshotListen: SnapshotListenInput<Time = EventTime<M, S>>,
{
    input: &'a Receiver<M>,
    registrations: &'a Receiver<crossbeam_channel::Sender<bool>>,
    snapshots: AHashMap<u128, SnapshotSlot<S, K, ApplyCompletion<M>, S::Rejection>>,
    listeners: NotificationCollections<EventTime<M, S>, <M::SnapshotListen as SnapshotListenInput>::Listener>,
    activity_listeners: Vec<crossbeam_channel::Sender<bool>>,
    events_config: S::Config,
    checkpoints_config: K::Config,
    checkpoints_context: <K as Checkpoints<S>>::Context,
    history_retention: EventTime<M, S>,
    current_time: EventTime<M, S>,
    horizon: EventTime<M, S>,
    dirty_snapshot_ids: Vec<u128>,
}

impl<'a, M, S, K> MessageWorker<'a, M, S, K>
where
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
    fn new(
        input: &'a Receiver<M>,
        registrations: &'a Receiver<crossbeam_channel::Sender<bool>>,
        events_config: S::Config,
        history_retention: EventTime<M, S>,
        checkpoints_config: K::Config,
        checkpoints_context: <K as Checkpoints<S>>::Context,
    ) -> Self {
        Self {
            input,
            registrations,
            snapshots: AHashMap::new(),
            listeners: NotificationCollections::new(),
            activity_listeners: Vec::new(),
            current_time: EventTime::<M, S>::default(),
            horizon: EventTime::<M, S>::default(),
            dirty_snapshot_ids: Vec::new(),
            events_config,
            history_retention,
            checkpoints_config,
            checkpoints_context,
        }
    }

    /// Waits and reports activity without consuming work.
    fn activity_loop(&mut self) {
        let mut ready = crossbeam_channel::Select::new();
        let work_ready = ready.recv(self.input);
        let subscription = ready.recv(self.registrations);
        loop {
            if ready.ready() != work_ready {
                match self.registrations.try_recv() {
                    Ok(listener) => {
                        if listener.send(false).is_ok() {
                            self.activity_listeners.push(listener);
                        }
                    }
                    Err(TryRecvError::Disconnected) => ready.remove(subscription),
                    Err(TryRecvError::Empty) => {}
                }
                continue;
            }
            self.activity_listeners.retain(|listener| listener.send(true).is_ok());
            if self.work_loop() == TryRecvError::Disconnected {
                return;
            }
            self.activity_listeners.retain(|listener| listener.send(false).is_ok());
        }
    }

    /// Drains available work; returns why receiving stopped.
    fn work_loop(&mut self) -> TryRecvError {
        let mut pending = None;
        loop {
            let message = match pending.take() {
                Some(message) => message,
                None => match self.input.try_recv() {
                    Ok(message) => message.into_kind(),
                    Err(reason) => return reason,
                },
            };
            match message {
                WorkInputKind::Apply(batch) => {
                    pending = self.apply_batch(batch);
                }
                WorkInputKind::SnapshotQuery(query) => {
                    query_snapshots(query, &self.snapshots, &self.checkpoints_config, &mut self.checkpoints_context)
                }
                WorkInputKind::EventQuery(query) => query_events::<_, ApplyEvent<M>, _, _, _, _>(query, &self.snapshots),
                WorkInputKind::SnapshotListen(registration) => {
                    let (time, snapshot_ids, listener) = registration.into_parts();
                    self.listeners.register(time, snapshot_ids, listener, &mut self.snapshots);
                }
                WorkInputKind::Advance(advance) => {
                    let (target_time, completion) = advance.into_parts();
                    crate::advance::advance_worker::<ApplyEvent<M>, _, _, _, _, _>(
                        &mut self.snapshots,
                        &self.checkpoints_config,
                        &mut self.checkpoints_context,
                        &mut self.current_time,
                        &mut self.horizon,
                        &self.history_retention,
                        target_time,
                        completion,
                        &mut |update, snapshots| self.listeners.record(update, snapshots),
                    );
                    self.listeners.flush();
                }
            }
        }
    }

    /// Coalesces applies and replays changed snapshots, preserving the next non-apply.
    fn apply_batch(&mut self, batch: M::Apply) -> Option<MessageKind<M>> {
        self.dirty_snapshot_ids.clear();
        insert_batch(batch, &mut self.snapshots, &mut self.dirty_snapshot_ids, &self.events_config, &self.horizon);
        let mut pending = None;
        while let Ok(message) = self.input.try_recv() {
            match message.into_kind() {
                WorkInputKind::Apply(batch) => {
                    insert_batch(batch, &mut self.snapshots, &mut self.dirty_snapshot_ids, &self.events_config, &self.horizon);
                }
                message => {
                    pending = Some(message);
                    break;
                }
            }
        }
        for snapshot_id in self.dirty_snapshot_ids.drain(..) {
            let update = update_snapshot(snapshot_id, &mut self.snapshots, &self.checkpoints_config, &mut self.checkpoints_context);
            self.listeners.record(update, &mut self.snapshots);
        }
        self.listeners.flush();
        pending
    }
}
#[cfg(test)]
mod tests {
    use ahash::AHashMap;
    use std::hint::black_box;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use criterion::{BatchSize, Criterion, SamplingMode, Throughput};
    use crossbeam_channel::{unbounded, Receiver};

    use super::{work, work_with_batch_limit};
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

    struct OrderedCheckpoints(u128);

    impl Checkpoints<TestEvents> for OrderedCheckpoints {
        type Config = ();
        type Context = Arc<Mutex<Vec<u128>>>;
        type Time = u64;

        fn create(snapshot_id: u128, _config: &()) -> Self {
            Self(snapshot_id)
        }

        fn update(&mut self, _events: &mut TestEvents, context: &mut Self::Context) -> Self::Time {
            context.lock().unwrap().push(self.0);
            0
        }

        fn advance_before(&mut self, _events: &TestEvents, _context: &mut Self::Context, _horizon: &u64) {}
    }

    struct SpaceTimeInput {
        time: u64,
        effect_count: usize,
    }

    #[derive(Default)]
    struct SpaceTimeEvents(Vec<SpaceTimeInput>);

    impl Events<SpaceTimeInput> for SpaceTimeEvents {
        type Config = ();
        type Rejection = ();
        type Time = u64;

        fn create(_id: u128, _config: &(), _horizon: &u64) -> Self {
            Self::default()
        }

        fn insert(&mut self, input: SpaceTimeInput) -> EventInsert<()> {
            self.0.push(input);
            EventInsert { changed: true, rejections: Vec::new() }
        }

        fn dirty_time(&self) -> &u64 {
            self.0.first().map_or(&0, |input| &input.time)
        }

        fn prune_before(&mut self, _horizon: &u64) {}
    }

    #[derive(Default)]
    struct SpaceTimeCheckpoints {
        applied: usize,
    }

    impl Checkpoints<SpaceTimeEvents> for SpaceTimeCheckpoints {
        type Config = ();
        type Context = Arc<AtomicUsize>;
        type Time = u64;

        fn create(_snapshot_id: u128, _config: &()) -> Self {
            Self::default()
        }

        fn update(&mut self, events: &mut SpaceTimeEvents, total_effects: &mut Self::Context) -> Self::Time {
            let mut effects = AHashMap::new();
            for input in &events.0[self.applied..] {
                for effect_index in 0..input.effect_count {
                    let effect_id = u128::from(input.time) << 64 | effect_index as u128;
                    effects.insert(effect_id, input.time);
                }
            }
            self.applied = events.0.len();
            total_effects.fetch_add(effects.len(), Ordering::Relaxed);
            events.0.last().map_or(0, |input| input.time)
        }

        fn advance_before(&mut self, _events: &SpaceTimeEvents, _context: &mut Self::Context, _horizon: &u64) {}
    }

    type TestCompletion = crossbeam_channel::Sender<Vec<()>>;

    fn batch(first_id: u128, count: u128) -> ApplyBatch<TestInput, TestCompletion> {
        let (completion, _responses) = unbounded();
        let inputs = (0..count).map(|offset| RoutedInput { snapshot_id: 7, input: TestInput(first_id + offset) }).collect();
        ApplyBatch { inputs, completion }
    }

    fn spacetime_batches(batch_count: usize, effect_count: usize) -> Receiver<ApplyBatch<SpaceTimeInput, TestCompletion>> {
        let (sender, receiver) = unbounded();
        for batch_index in 0..batch_count {
            let (completion, _responses) = unbounded();
            sender
                .send(ApplyBatch {
                    inputs: vec![RoutedInput { snapshot_id: 7, input: SpaceTimeInput { time: batch_index as u64 + 1, effect_count } }],
                    completion,
                })
                .unwrap();
        }
        drop(sender);
        receiver
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
    fn one_batch_replays_changed_snapshots_in_first_seen_order() {
        let (sender, receiver) = unbounded();
        let context = Arc::new(Mutex::new(Vec::new()));
        let (completion, _responses) = unbounded();
        sender
            .send(ApplyBatch {
                inputs: vec![
                    RoutedInput { snapshot_id: 7, input: TestInput(1) },
                    RoutedInput { snapshot_id: 9, input: TestInput(2) },
                    RoutedInput { snapshot_id: 9, input: TestInput(3) },
                ],
                completion,
            })
            .unwrap();
        drop(sender);

        let mut worker_config = config(1);
        worker_config.maximum_dirty_age = Duration::from_secs(60);
        work::<_, TestEvents, OrderedCheckpoints>(receiver, worker_config, (), (), Arc::clone(&context));

        assert_eq!(*context.lock().unwrap(), vec![7, 9]);
    }

    #[test]
    fn ready_batches_are_inserted_before_one_checkpoint_update() {
        let (sender, receiver) = unbounded();
        let context = Arc::new(Mutex::new(Vec::new()));
        sender.send(batch(1, 1)).unwrap();
        sender.send(batch(2, 1)).unwrap();
        drop(sender);

        work::<_, TestEvents, TestCheckpoints>(receiver, config(1), (), (), Arc::clone(&context));

        assert_eq!(*context.lock().unwrap(), vec![vec![1, 2]]);
    }

    #[test]
    fn one_batch_cycle_limit_preserves_one_replay_per_ready_batch() {
        let (sender, receiver) = unbounded();
        let context = Arc::new(Mutex::new(Vec::new()));
        sender.send(batch(1, 1)).unwrap();
        sender.send(batch(2, 1)).unwrap();
        drop(sender);

        work_with_batch_limit::<_, TestEvents, TestCheckpoints>(receiver, config(1), (), (), Arc::clone(&context), 1);

        assert_eq!(*context.lock().unwrap(), vec![vec![1], vec![1, 2]]);
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

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_spacetime_shaped_replay_cycles() {
        let mut criterion =
            Criterion::default().sample_size(10).warm_up_time(Duration::from_millis(100)).measurement_time(Duration::from_secs(2));

        for batch_count in [2, 200] {
            let effect_count = 1_000;
            let mut group = criterion.benchmark_group(format!("worker/work/spacetime_shape/{batch_count}_batches/{effect_count}_effects"));
            group.sampling_mode(SamplingMode::Flat);
            group.throughput(Throughput::Elements((batch_count * effect_count) as u64));
            for (name, maximum_batches_per_cycle) in [("one_batch_cycles", 1), ("ready_batch_cycle", usize::MAX)] {
                group.bench_function(name, |bencher| {
                    bencher.iter_batched(
                        || spacetime_batches(batch_count, effect_count),
                        |receiver| {
                            let total_effects = Arc::new(AtomicUsize::new(0));
                            work_with_batch_limit::<_, SpaceTimeEvents, SpaceTimeCheckpoints>(
                                receiver,
                                config(1),
                                (),
                                (),
                                Arc::clone(&total_effects),
                                maximum_batches_per_cycle,
                            );
                            black_box(total_effects.load(Ordering::Relaxed));
                        },
                        BatchSize::LargeInput,
                    );
                });
            }
            group.finish();
        }
        criterion.final_summary();
    }
}
