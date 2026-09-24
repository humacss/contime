use ahash::AHashMap;

use crate::types::StoreSlot;
use crate::{EventQueryInput, EventQueryResponse, SnapshotQueryInput, SnapshotQueryResponse, SnapshotStore};

pub(crate) fn query_snapshots<Q, I, S>(query: Q, snapshots: &mut AHashMap<u128, StoreSlot<S>>, context: &mut S::Context)
where
    Q: SnapshotQueryInput<Time = S::Time>,
    Q::Response: SnapshotQueryResponse<S::Snapshot>,
    S: SnapshotStore<I>,
{
    let (time, snapshot_ids, response) = query.into_parts();
    let mut results = Vec::new();
    for snapshot_id in snapshot_ids {
        let Some(store) = snapshots.get_mut(&snapshot_id).and_then(|slot| slot.store.as_mut()) else { continue };
        if let Some(snapshot) = store.query(time.clone(), context) {
            results.push(snapshot);
        }
    }
    if !results.is_empty() {
        response.send(results);
    }
}

pub(crate) fn query_events<Q, I, S>(query: Q, snapshots: &AHashMap<u128, StoreSlot<S>>)
where
    Q: EventQueryInput<Time = S::Time>,
    Q::Response: EventQueryResponse<I>,
    I: Clone,
    S: SnapshotStore<I>,
{
    let (snapshot_id, from, to, response) = query.into_parts();
    let Some(store) = snapshots.get(&snapshot_id).and_then(|slot| slot.store.as_ref()) else { return };
    let events = store.query_events(&from, &to);
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
        work_messages, AdvanceInput, ApplyBatch, EventInsert, EventQueryInput, RoutedInput, SnapshotListenInput, SnapshotListener,
        SnapshotQueryInput, SnapshotStore, WorkInput, WorkInputKind, WorkerConfig,
    };

    #[derive(Clone)]
    struct TestEvent(u64);

    #[derive(Default)]
    struct TestStore {
        events: Vec<TestEvent>,
        tip: Option<u64>,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestSnapshot {
        snapshot_id: u128,
        count: usize,
    }

    impl SnapshotStore<TestEvent> for TestStore {
        type Config = ();
        type Context = ();
        type Time = u64;
        type Snapshot = TestSnapshot;
        type Rejection = ();

        fn create(_: u128, _: &(), _: &u64) -> Self {
            Self::default()
        }
        fn insert(&mut self, input: TestEvent, horizon: &u64) -> EventInsert<()> {
            if input.0 < *horizon {
                return EventInsert { changed: false, rejections: vec![()] };
            }
            self.events.push(input);
            self.tip = None;
            EventInsert { changed: true, rejections: vec![] }
        }
        fn event_time(input: &TestEvent) -> u64 {
            input.0
        }
        fn earliest_replay_time(&self) -> Option<u64> {
            self.events.iter().filter(|event| self.tip.is_none_or(|time| event.0 > time)).map(|event| event.0).min()
        }
        fn process_until(&mut self, target: &u64, _: &mut ()) {
            self.tip = Some(*target);
        }
        fn query(&mut self, _: u64, _: &mut ()) -> Option<Box<TestSnapshot>> {
            Some(Box::new(TestSnapshot { snapshot_id: 7, count: self.events.len() }))
        }
        fn query_events(&self, from: &u64, to: &u64) -> Vec<TestEvent> {
            self.events.iter().filter(|event| from <= &event.0 && &event.0 < to).cloned().collect()
        }
        fn forward(&mut self, _: &u64, _: &mut ()) {}
        fn prune(&mut self) {}
    }

    type Completion = Sender<Vec<()>>;

    // Match the worker response contract, which transfers boxed snapshots.
    #[allow(clippy::vec_box)]
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
        Prune(Advance),
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
                Self::Prune(advance) => WorkInputKind::Prune(advance),
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

    struct PruningStore(TestStore);

    struct PruningContext {
        input: Option<Sender<Message>>,
        observed: Sender<&'static str>,
        completion: crossbeam_channel::Receiver<()>,
        fail: bool,
    }

    impl SnapshotStore<TestEvent> for PruningStore {
        type Config = ();
        type Context = PruningContext;
        type Time = u64;
        type Snapshot = TestSnapshot;
        type Rejection = ();

        fn create(_: u128, _: &(), _: &u64) -> Self {
            Self(TestStore::default())
        }
        fn insert(&mut self, event: TestEvent, horizon: &u64) -> EventInsert<()> {
            self.0.insert(event, horizon)
        }
        fn event_time(input: &TestEvent) -> u64 {
            input.0
        }
        fn earliest_replay_time(&self) -> Option<u64> {
            self.0.earliest_replay_time()
        }
        fn process_until(&mut self, time: &u64, _: &mut PruningContext) {
            self.0.process_until(time, &mut ());
        }
        fn forward(&mut self, _: &u64, context: &mut PruningContext) {
            assert!(!context.fail, "retention hook failed");
            context.observed.send("prune").unwrap();
            if let Some(input) = context.input.take() {
                let (response, _) = unbounded();
                input.send(Message::Snapshots(SnapshotQuery { response })).unwrap();
            }
        }
        fn prune(&mut self) {}
        fn query(&mut self, time: u64, context: &mut PruningContext) -> Option<Box<TestSnapshot>> {
            assert_eq!(context.completion.try_recv(), Err(crossbeam_channel::TryRecvError::Empty));
            context.observed.send("query").unwrap();
            self.0.query(time, &mut ())
        }
        fn query_events(&self, from: &u64, to: &u64) -> Vec<TestEvent> {
            self.0.query_events(from, to)
        }
    }

    #[test]
    fn pruning_services_queries_between_snapshots_and_finishes_before_shutdown() {
        let (input, receiver) = unbounded();
        let (observed, observations) = unbounded();
        let (prune_completion, done) = unbounded();
        let reports = observed.clone();
        let context = PruningContext { input: Some(input.clone()), observed, completion: done.clone(), fail: false };
        let coordination = crate::Coordination {
            router_count: 1,
            report: Box::new(|_, _| {}),
            pruned: Box::new(move |horizon| {
                assert_eq!(horizon, 1);
                reports.send("completed").unwrap();
            }),
        };
        let (completion, _) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: [7, 8, 9].map(|snapshot_id| RoutedInput { snapshot_id, input: TestEvent(10) }).into(),
                completion,
            }))
            .unwrap();
        input.send(Message::Prune(Advance { time: 1, completion: prune_completion })).unwrap();
        drop(input);
        work_messages::<_, PruningStore>(receiver, config(), (), 0, context, crossbeam_channel::never(), Some(coordination));
        assert_eq!(done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
        let actual = observations.try_iter().collect::<Vec<_>>();
        assert_eq!(actual, ["prune", "query", "prune", "prune", "completed"]);
    }

    #[test]
    fn overlapping_prunes_finish_in_order_and_repeated_horizons_do_not_prune_again() {
        let (input, receiver) = unbounded();
        let (observed, observations) = unbounded();
        let (completion, done) = unbounded();
        let context = PruningContext { input: Some(input.clone()), observed, completion: done.clone(), fail: false };
        let (applied, _) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: [7, 8, 9].map(|snapshot_id| RoutedInput { snapshot_id, input: TestEvent(10) }).into(),
                completion: applied,
            }))
            .unwrap();
        for time in [1, 2, 2] {
            input.send(Message::Prune(Advance { time, completion: completion.clone() })).unwrap();
        }
        drop(completion);
        drop(input);
        work_messages::<_, PruningStore>(receiver, config(), (), 0, context, crossbeam_channel::never(), None);
        assert_eq!(done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
        assert_eq!(observations.try_iter().collect::<Vec<_>>(), ["prune", "query", "prune", "prune", "prune", "prune", "prune"]);
    }

    #[test]
    fn future_only_history_does_not_replay_before_explicit_advance() {
        let (input, receiver) = unbounded();
        let (notifications, observed) = unbounded();
        input.send(Message::Listen(Listen { time: 10, snapshot_ids: vec![7], listener: TestListener(notifications) })).unwrap();
        let (completion, done) = unbounded();
        input.send(Message::Apply(ApplyBatch { inputs: vec![RoutedInput { snapshot_id: 7, input: TestEvent(1) }], completion })).unwrap();
        drop(input);

        work_messages::<_, TestStore>(receiver, config(), (), 0, (), crossbeam_channel::never(), None);

        assert_eq!(observed.try_iter().collect::<Vec<_>>(), vec![ListenerMessage::Registered { time: 10, snapshot_ids: vec![7] }]);
        assert_eq!(done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
    }

    #[test]
    fn failed_pruning_does_not_report_success_when_completion_sender_drops() {
        let (input, receiver) = unbounded();
        let (observed, _) = unbounded();
        let (completion, done) = unbounded();
        let (pruned, reports) = unbounded();
        let context = PruningContext { input: None, observed, completion: done.clone(), fail: true };
        let coordination = crate::Coordination {
            router_count: 1,
            report: Box::new(|_, _| {}),
            pruned: Box::new(move |horizon| {
                pruned.send(horizon).unwrap();
            }),
        };
        input
            .send(Message::Apply(ApplyBatch {
                inputs: vec![RoutedInput { snapshot_id: 7, input: TestEvent(10) }],
                completion: unbounded().0,
            }))
            .unwrap();
        input.send(Message::Prune(Advance { time: 1, completion })).unwrap();
        drop(input);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            work_messages::<_, PruningStore>(receiver, config(), (), 0, context, crossbeam_channel::never(), Some(coordination));
        }));
        assert!(result.is_err());
        assert_eq!(done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
        assert_eq!(reports.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
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

        work_messages::<_, TestStore>(receiver, config(), (), 0, (), crossbeam_channel::never(), None);

        assert_eq!(*snapshots.recv().unwrap()[0], TestSnapshot { snapshot_id: 7, count: 3 });
        assert_eq!(events.recv().unwrap().into_iter().map(|event| event.0).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn one_worker_queue_completes_horizon_advancement() {
        let (input, receiver) = unbounded();
        let (completion, done) = unbounded();
        input.send(Message::Advance(Advance { time: 20, completion })).unwrap();
        drop(input);

        work_messages::<_, TestStore>(receiver, config(), (), 10, (), crossbeam_channel::never(), None);

        assert_eq!(done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
    }

    #[test]
    fn one_worker_queue_registers_before_snapshot_creation_and_notifies_after_replay() {
        let (input, receiver) = unbounded();
        let (notifications, observed) = unbounded();
        input.send(Message::Listen(Listen { time: 10, snapshot_ids: vec![7], listener: TestListener(notifications) })).unwrap();
        input.send(Message::Advance(Advance { time: 10, completion: unbounded().0 })).unwrap();
        let (completion, _rejections) = unbounded();
        input.send(Message::Apply(ApplyBatch { inputs: vec![RoutedInput { snapshot_id: 7, input: TestEvent(1) }], completion })).unwrap();
        drop(input);

        work_messages::<_, TestStore>(receiver, config(), (), 0, (), crossbeam_channel::never(), None);

        assert_eq!(
            observed.try_iter().collect::<Vec<_>>(),
            vec![
                ListenerMessage::Registered { time: 10, snapshot_ids: vec![7] },
                ListenerMessage::Replayed { time: 10, snapshot_ids: vec![7] },
            ]
        );
    }

    #[test]
    fn worker_flushes_notifications_between_snapshot_steps() {
        let (input, receiver) = unbounded();
        let (notifications, observed) = unbounded();
        let snapshot_ids = (0..100_u128).collect::<Vec<_>>();
        input
            .send(Message::Listen(Listen { time: 10, snapshot_ids: snapshot_ids.clone(), listener: TestListener(notifications) }))
            .unwrap();
        input.send(Message::Advance(Advance { time: 10, completion: unbounded().0 })).unwrap();
        let (completion, _rejections) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: snapshot_ids.iter().map(|&snapshot_id| RoutedInput { snapshot_id, input: TestEvent(1) }).collect(),
                completion,
            }))
            .unwrap();
        drop(input);

        work_messages::<_, TestStore>(receiver, config(), (), 0, (), crossbeam_channel::never(), None);

        assert_eq!(observed.recv().unwrap(), ListenerMessage::Registered { time: 10, snapshot_ids: snapshot_ids.clone() });
        for snapshot_id in snapshot_ids {
            assert_eq!(observed.recv().unwrap(), ListenerMessage::Replayed { time: 10, snapshot_ids: vec![snapshot_id] });
        }
        assert!(observed.try_recv().is_err());
    }

    #[test]
    fn snapshot_steps_ignore_the_old_replay_budget() {
        let (input, receiver) = unbounded();
        let (notifications, observed) = unbounded();
        input.send(Message::Listen(Listen { time: 10, snapshot_ids: vec![1, 2, 3], listener: TestListener(notifications) })).unwrap();
        input.send(Message::Advance(Advance { time: 10, completion: unbounded().0 })).unwrap();
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

        work_messages::<_, TestStore>(receiver, worker_config, (), 0, (), crossbeam_channel::never(), None);

        assert!(matches!(observed.recv().unwrap(), ListenerMessage::Registered { .. }));
        for snapshot_id in [1, 2, 3] {
            assert_eq!(observed.recv().unwrap(), ListenerMessage::Replayed { time: 10, snapshot_ids: vec![snapshot_id] });
        }
        assert!(observed.try_recv().is_err());
    }

    #[test]
    fn adjacent_applies_share_processing_and_complete_independently() {
        let (input, receiver) = unbounded();
        let (notifications, observed) = unbounded();
        input.send(Message::Listen(Listen { time: 10, snapshot_ids: vec![7], listener: TestListener(notifications) })).unwrap();
        input.send(Message::Advance(Advance { time: 10, completion: unbounded().0 })).unwrap();

        let (first_completion, first_done) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: vec![RoutedInput { snapshot_id: 7, input: TestEvent(1) }],
                completion: first_completion,
            }))
            .unwrap();
        let (second_completion, second_done) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: vec![RoutedInput { snapshot_id: 7, input: TestEvent(2) }],
                completion: second_completion,
            }))
            .unwrap();
        drop(input);

        work_messages::<_, TestStore>(receiver, config(), (), 0, (), crossbeam_channel::never(), None);

        assert!(matches!(observed.recv().unwrap(), ListenerMessage::Registered { .. }));
        assert_eq!(observed.recv().unwrap(), ListenerMessage::Replayed { time: 10, snapshot_ids: vec![7] });
        assert!(observed.try_recv().is_err());
        assert_eq!(first_done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
        assert_eq!(second_done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
    }

    #[test]
    fn a_query_is_an_apply_cycle_barrier() {
        let (input, receiver) = unbounded();
        let (first_completion, _first_done) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: vec![RoutedInput { snapshot_id: 7, input: TestEvent(1) }],
                completion: first_completion,
            }))
            .unwrap();
        let (snapshot_response, snapshots) = unbounded();
        input.send(Message::Snapshots(SnapshotQuery { response: snapshot_response })).unwrap();
        let (second_completion, _second_done) = unbounded();
        input
            .send(Message::Apply(ApplyBatch {
                inputs: vec![RoutedInput { snapshot_id: 7, input: TestEvent(2) }],
                completion: second_completion,
            }))
            .unwrap();
        drop(input);

        work_messages::<_, TestStore>(receiver, config(), (), 0, (), crossbeam_channel::never(), None);

        assert_eq!(*snapshots.recv().unwrap()[0], TestSnapshot { snapshot_id: 7, count: 1 });
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
            crate::types::StoreSlot {
                store: Some(TestStore { events: (0..1_000).map(TestEvent).collect(), tip: None }),
                notification_ids: Vec::new(),
            },
        );
        let mut criterion = Criterion::default();

        criterion.bench_function("worker/query/snapshot/one_found", |bencher| {
            bencher.iter(|| super::query_snapshots::<_, TestEvent, _>(BenchmarkSnapshotQuery, black_box(&mut snapshots), &mut ()));
        });
        criterion.bench_function("worker/query/events/1000_found", |bencher| {
            bencher.iter(|| super::query_events::<_, TestEvent, _>(BenchmarkEventQuery, black_box(&snapshots)));
        });
        criterion.final_summary();
    }
}
