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
        let result = if let Some(checkpoints) = slot.checkpoints.as_ref() {
            checkpoints.query_at(&slot.events, checkpoints_context, time.clone())
        } else {
            K::create(snapshot_id, checkpoints_config).query_at(&slot.events, checkpoints_context, time.clone())
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
    let events = slot.events.clone_between(&from, &to);
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
        RoutedInput, SnapshotQueryInput, WorkInput, WorkInputKind, WorkerConfig,
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

        fn update(&mut self, _events: &mut TestEvents, _context: &mut Self::Context) {}

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
        Advance(Advance),
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
        type Advance = Advance;

        fn into_kind(self) -> WorkInputKind<ApplyBatch<TestEvent, Completion>, SnapshotQuery, EventQuery, Advance> {
            match self {
                Self::Apply(batch) => WorkInputKind::Apply(batch),
                Self::Snapshots(query) => WorkInputKind::SnapshotQuery(query),
                Self::Events(query) => WorkInputKind::EventQuery(query),
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
                events: TestEvents((0..1_000).map(TestEvent).collect()),
                checkpoints: Some(TestCheckpoints),
                waiters: Vec::new(),
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
