use ahash::AHashMap;

use crate::types::SnapshotSlot;
use crate::{Checkpoints, EventQueryInput, EventQueryResponse, QueryCheckpoints, QueryEvents, SnapshotQueryInput, SnapshotQueryResponse};

pub(crate) fn query_snapshots<Q, S, K, C, R>(
    query: Q,
    snapshots: &AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    checkpoints_config: &K::Config,
    checkpoints_context: &mut <K as Checkpoints<S>>::Context,
) where
    Q: SnapshotQueryInput<Time = <K as QueryCheckpoints<S>>::Time>,
    Q::Response: SnapshotQueryResponse<<K as QueryCheckpoints<S>>::Snapshot>,
    K: Checkpoints<S> + QueryCheckpoints<S, Context = <K as Checkpoints<S>>::Context>,
{
    let (time, snapshot_ids, response) = query.into_parts();
    let mut results = Vec::new();
    for snapshot_id in snapshot_ids {
        let Some(slot) = snapshots.get(&snapshot_id) else { continue };
        let Some(events) = slot.events.as_ref() else { continue };
        let result = if let Some(checkpoints) = slot.checkpoints.as_ref() {
            checkpoints.query_at(events, checkpoints_context, time.clone())
        } else {
            K::create(snapshot_id, checkpoints_config).query_at(events, checkpoints_context, time.clone())
        };
        if let Some(snapshot) = result {
            results.push(snapshot);
        }
    }

    if !results.is_empty() {
        response.send(results);
    }
}

pub(crate) fn query_events<Q, I, S, K, C, R>(query: Q, snapshots: &AHashMap<u128, SnapshotSlot<S, K, C, R>>)
where
    Q: EventQueryInput<Time = <S as QueryEvents<I>>::Time>,
    Q::Response: EventQueryResponse<I>,
    I: Clone,
    S: QueryEvents<I>,
{
    let (snapshot_id, from, to, response) = query.into_parts();
    let Some(slot) = snapshots.get(&snapshot_id) else { return };
    let Some(history) = slot.events.as_ref() else { return };
    let events = history.clone_between(&from, &to);
    if !events.is_empty() {
        response.send(events);
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::Duration;

    use ahash::AHashMap;
    use criterion::Criterion;
    use crossbeam_channel::{unbounded, Sender};

    use crate::{
        work_messages, AdvanceInput, ApplyBatch, Checkpoints, EventInsert, EventQueryInput, Events, QueryCheckpoints, QueryEvents,
        RoutedInput, SnapshotListenInput, SnapshotListener, SnapshotQueryInput, WorkInput, WorkInputKind, WorkerConfig,
    };

    #[derive(Clone)]
    struct TestEvent(u64);

    #[derive(Default)]
    struct TestEvents(Vec<TestEvent>);

    impl Events<TestEvent> for TestEvents {
        type Config = ();
        type Rejection = ();
        type Time = u64;

        fn create(_snapshot_id: u128, _config: &Self::Config, _horizon: &u64) -> Self {
            Self::default()
        }

        fn insert(&mut self, input: TestEvent) -> EventInsert<Self::Rejection> {
            self.0.push(input);
            EventInsert { changed: true, rejections: Vec::new() }
        }

        fn dirty_time(&self) -> &u64 {
            &0
        }

        fn prune_before(&mut self, _horizon: &u64) {}
    }

    impl QueryEvents<TestEvent> for TestEvents {
        type Time = u64;

        fn clone_between(&self, from: &Self::Time, to: &Self::Time) -> Vec<TestEvent>
        where
            TestEvent: Clone,
        {
            self.0.iter().filter(|event| from <= &event.0 && &event.0 < to).cloned().collect()
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestSnapshot {
        snapshot_id: u128,
        count: usize,
    }

    struct TestCheckpoints;

    impl Checkpoints<TestEvents> for TestCheckpoints {
        type Config = ();
        type Context = ();
        type Time = u64;
        fn create(_snapshot_id: u128, _config: &Self::Config) -> Self {
            Self
        }

        fn update(&mut self, _events: &mut TestEvents, _context: &mut Self::Context) -> Self::Time {
            0
        }

        fn advance_before(&mut self, _events: &TestEvents, _context: &mut Self::Context, _horizon: &u64) {}
    }

    impl QueryCheckpoints<TestEvents> for TestCheckpoints {
        type Context = ();
        type Time = u64;
        type Snapshot = TestSnapshot;

        fn query_at(&self, events: &TestEvents, _context: &mut Self::Context, _time: Self::Time) -> Option<Box<Self::Snapshot>> {
            Some(Box::new(TestSnapshot { snapshot_id: 7, count: events.0.len() }))
        }
    }

    type Completion = Sender<Vec<()>>;

    struct SnapshotQuery {
        response: Sender<Vec<Box<TestSnapshot>>>,
    }

    impl SnapshotQueryInput for SnapshotQuery {
        type Time = u64;
        type Response = Sender<Vec<Box<TestSnapshot>>>;

        fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response) {
            (10, vec![7, 999], self.response)
        }
    }

    struct EventQuery {
        response: Sender<Vec<TestEvent>>,
    }

    impl EventQueryInput for EventQuery {
        type Time = u64;
        type Response = Sender<Vec<TestEvent>>;

        fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response) {
            (7, 1, 3, self.response)
        }
    }

    enum Message {
        Apply(ApplyBatch<TestEvent, Completion>),
        Snapshots(SnapshotQuery),
        Events(EventQuery),
        Listen(Listen),
        Advance(Advance),
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum ListenerMessage {
        Registered { time: u64, snapshot_ids: Vec<u128> },
        Replayed { time: u64, snapshot_ids: Vec<u128> },
    }

    #[derive(Clone)]
    struct TestListener(Sender<ListenerMessage>);

    impl SnapshotListener<u64> for TestListener {
        fn registered(&self, time: u64, snapshot_ids: Vec<u128>) -> bool {
            self.0.send(ListenerMessage::Registered { time, snapshot_ids }).is_ok()
        }

        fn replayed(&self, time: u64, snapshot_ids: Vec<u128>) -> bool {
            self.0.send(ListenerMessage::Replayed { time, snapshot_ids }).is_ok()
        }
    }

    struct Listen {
        time: u64,
        snapshot_ids: Vec<u128>,
        listener: TestListener,
    }

    impl SnapshotListenInput for Listen {
        type Time = u64;
        type Listener = TestListener;

        fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener) {
            (self.time, self.snapshot_ids, self.listener)
        }
    }

    struct Advance {
        time: u64,
        completion: Sender<()>,
    }

    impl AdvanceInput for Advance {
        type Time = u64;
        type Completion = Sender<()>;

        fn into_parts(self) -> (u64, Sender<()>) {
            (self.time, self.completion)
        }
    }

    impl WorkInput for Message {
        type Apply = ApplyBatch<TestEvent, Completion>;
        type SnapshotQuery = SnapshotQuery;
        type EventQuery = EventQuery;
        type SnapshotListen = Listen;
        type Advance = Advance;

        fn into_kind(self) -> WorkInputKind<ApplyBatch<TestEvent, Completion>, SnapshotQuery, EventQuery, Listen, Advance> {
            match self {
                Self::Apply(batch) => WorkInputKind::Apply(batch),
                Self::Snapshots(query) => WorkInputKind::SnapshotQuery(query),
                Self::Events(query) => WorkInputKind::EventQuery(query),
                Self::Listen(listen) => WorkInputKind::SnapshotListen(listen),
                Self::Advance(advance) => WorkInputKind::Advance(advance),
            }
        }
    }

    fn config() -> WorkerConfig {
        WorkerConfig {
            maximum_dirty_age: Duration::from_micros(100),
            replays_per_receive: 0,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        }
    }

    #[test]
    fn one_worker_queue_serves_snapshot_and_event_queries_without_forced_replay() {
        let (input, receiver) = unbounded();
        let (completion, _rejections) = unbounded();
        let (snapshot_response, snapshots) = unbounded();
        let (event_response, events) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: vec![
                    RoutedInput { snapshot_id: 7, input: TestEvent(1) },
                    RoutedInput { snapshot_id: 7, input: TestEvent(2) },
                    RoutedInput { snapshot_id: 7, input: TestEvent(3) },
                ],
                completion,
            }))
            .unwrap();
        input.send(Message::Snapshots(SnapshotQuery { response: snapshot_response })).unwrap();
        input.send(Message::Events(EventQuery { response: event_response })).unwrap();
        drop(input);

        work_messages::<_, TestEvents, TestCheckpoints>(receiver, config(), (), 0, (), ());

        assert_eq!(*snapshots.recv().unwrap()[0], TestSnapshot { snapshot_id: 7, count: 3 });
        assert_eq!(events.recv().unwrap().into_iter().map(|event| event.0).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn one_worker_queue_completes_horizon_advancement() {
        let (input, receiver) = unbounded();
        let (completion, done) = unbounded();
        input.send(Message::Advance(Advance { time: 20, completion })).unwrap();
        drop(input);

        work_messages::<_, TestEvents, TestCheckpoints>(receiver, config(), (), 10, (), ());

        assert_eq!(done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
    }

    #[test]
    fn one_worker_queue_registers_before_snapshot_creation_and_notifies_after_replay() {
        let (input, receiver) = unbounded();
        let (notifications, observed) = unbounded();
        input.send(Message::Listen(Listen { time: 0, snapshot_ids: vec![7], listener: TestListener(notifications) })).unwrap();
        let (completion, _rejections) = unbounded();
        input.send(Message::Apply(ApplyBatch { inputs: vec![RoutedInput { snapshot_id: 7, input: TestEvent(1) }], completion })).unwrap();
        drop(input);

        work_messages::<_, TestEvents, TestCheckpoints>(receiver, config(), (), 0, (), ());

        assert_eq!(
            observed.try_iter().collect::<Vec<_>>(),
            vec![
                ListenerMessage::Registered { time: 0, snapshot_ids: vec![7] },
                ListenerMessage::Replayed { time: 0, snapshot_ids: vec![7] },
            ]
        );
    }

    #[test]
    fn one_worker_replay_batch_sends_one_notification_for_one_hundred_snapshots() {
        let (input, receiver) = unbounded();
        let (notifications, observed) = unbounded();
        let snapshot_ids = (0..100_u128).collect::<Vec<_>>();
        input.send(Message::Listen(Listen { time: 0, snapshot_ids: snapshot_ids.clone(), listener: TestListener(notifications) })).unwrap();
        let (completion, _rejections) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: snapshot_ids.iter().map(|&snapshot_id| RoutedInput { snapshot_id, input: TestEvent(1) }).collect(),
                completion,
            }))
            .unwrap();
        drop(input);

        work_messages::<_, TestEvents, TestCheckpoints>(receiver, config(), (), 0, (), ());

        assert_eq!(observed.recv().unwrap(), ListenerMessage::Registered { time: 0, snapshot_ids: snapshot_ids.clone() });
        let ListenerMessage::Replayed { time, snapshot_ids: mut replayed } = observed.recv().unwrap() else {
            panic!("expected replay notification")
        };
        replayed.sort_unstable();
        assert_eq!(time, 0);
        assert_eq!(replayed, snapshot_ids);
        assert!(observed.try_recv().is_err());
    }

    #[test]
    fn replay_budget_sends_deferred_snapshots_in_a_later_notification() {
        let (input, receiver) = unbounded();
        let (notifications, observed) = unbounded();
        input.send(Message::Listen(Listen { time: 0, snapshot_ids: vec![1, 2, 3], listener: TestListener(notifications) })).unwrap();
        let (completion, _rejections) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: (1..=3).map(|snapshot_id| RoutedInput { snapshot_id, input: TestEvent(1) }).collect(),
                completion,
            }))
            .unwrap();
        drop(input);
        let mut worker_config = config();
        worker_config.replays_per_receive = 1;

        work_messages::<_, TestEvents, TestCheckpoints>(receiver, worker_config, (), 0, (), ());

        assert!(matches!(observed.recv().unwrap(), ListenerMessage::Registered { .. }));
        let first = observed.recv().unwrap();
        let second = observed.recv().unwrap();
        let ListenerMessage::Replayed { snapshot_ids: first, .. } = first else { panic!("expected replay") };
        let ListenerMessage::Replayed { snapshot_ids: second, .. } = second else { panic!("expected replay") };
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 2);
        let mut replayed = first.into_iter().chain(second).collect::<Vec<_>>();
        replayed.sort_unstable();
        assert_eq!(replayed, vec![1, 2, 3]);
    }

    struct BenchmarkSnapshotQuery;

    impl SnapshotQueryInput for BenchmarkSnapshotQuery {
        type Time = u64;
        type Response = ();

        fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response) {
            (1_000, vec![7], ())
        }
    }

    struct BenchmarkEventQuery;

    impl EventQueryInput for BenchmarkEventQuery {
        type Time = u64;
        type Response = ();

        fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response) {
            (7, 0, 1_000, ())
        }
    }

    impl crate::SnapshotQueryResponse<TestSnapshot> for () {
        fn send(self, snapshots: Vec<Box<TestSnapshot>>) {
            black_box(snapshots);
        }
    }

    impl crate::EventQueryResponse<TestEvent> for () {
        fn send(self, events: Vec<TestEvent>) {
            black_box(events);
        }
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_query() {
        let mut snapshots = AHashMap::new();
        snapshots.insert(
            7,
            crate::types::SnapshotSlot::<TestEvents, TestCheckpoints, Completion, ()> {
                events: Some(TestEvents((0..1_000).map(TestEvent).collect())),
                checkpoints: Some(TestCheckpoints),
                waiters: Vec::new(),
                notification_ids: Vec::new(),
            },
        );
        let mut criterion = Criterion::default();

        criterion.bench_function("worker/query/snapshot/one_found", |bencher| {
            bencher.iter(|| super::query_snapshots(BenchmarkSnapshotQuery, black_box(&snapshots), &(), &mut ()));
        });
        criterion.bench_function("worker/query/events/1000_found", |bencher| {
            bencher.iter(|| super::query_events::<_, TestEvent, _, _, _, _>(BenchmarkEventQuery, black_box(&snapshots)));
        });
        criterion.final_summary();
    }
}
