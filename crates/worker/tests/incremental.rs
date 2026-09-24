use std::thread::JoinHandle;
use std::time::Duration;

use contime_worker::{
    work_messages, AdvanceInput, ApplyBatch, Coordination, EventInsert, EventQueryInput, RoutedInput, SnapshotListenInput,
    SnapshotListener, SnapshotQueryInput, SnapshotStore, WorkInput, WorkInputKind, WorkerConfig,
};
use crossbeam_channel::{unbounded, Receiver, Sender};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Event {
    id: u128,
    time: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    id: u128,
    time: u64,
    ids: Vec<u128>,
}
struct Context {
    steps: Sender<Snapshot>,
    gate: Option<Receiver<()>>,
}

// A storage stub: these tests exercise worker scheduling, not checkpoint replay.
struct TestStore {
    id: u128,
    events: Vec<Event>,
    processed: Option<u64>,
    horizon: u64,
    prefix: Vec<u128>,
}
impl SnapshotStore<Event> for TestStore {
    type Config = ();
    type Context = Context;
    type Time = u64;
    type Snapshot = Snapshot;
    type Rejection = u128;

    fn create(id: u128, _: &(), horizon: &u64) -> Self {
        Self { id, events: vec![], processed: None, horizon: *horizon, prefix: vec![] }
    }
    fn insert(&mut self, event: Event, horizon: &u64) -> EventInsert<u128> {
        if event.time < *horizon {
            return EventInsert { changed: false, rejections: vec![event.id] };
        }
        if self.events.iter().any(|existing| existing.id == event.id) {
            return EventInsert { changed: false, rejections: vec![] };
        }
        if self.processed.is_some_and(|time| time >= event.time) {
            self.processed = event.time.checked_sub(1);
        }
        self.events.push(event);
        self.events.sort_by_key(|event| (event.time, event.id));
        EventInsert { changed: true, rejections: vec![] }
    }
    fn event_time(event: &Event) -> u64 {
        event.time
    }
    fn process_until(&mut self, time: &u64, context: &mut Context) {
        let snapshot = self.query(*time, context).unwrap();
        self.processed = Some(*time);
        context.steps.send(*snapshot).unwrap();
        if let Some(gate) = &context.gate {
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }
    fn query(&mut self, time: u64, _: &mut Context) -> Option<Box<Snapshot>> {
        let ids = self.prefix.iter().copied().chain(self.events.iter().filter(|event| event.time <= time).map(|event| event.id)).collect();
        Some(Box::new(Snapshot { id: self.id, time, ids }))
    }
    fn query_events(&self, from: &u64, to: &u64) -> Vec<Event> {
        self.events.iter().filter(|event| event.time >= *from && event.time < *to).cloned().collect()
    }
    fn forward(&mut self, horizon: &u64, _: &mut Context) {
        self.horizon = *horizon;
    }
    fn prune(&mut self) {
        let count = self.events.partition_point(|event| event.time < self.horizon);
        self.prefix.extend(self.events.drain(..count).map(|event| event.id));
    }
}

// Match the boxed-snapshot response contract.
#[allow(clippy::vec_box)]
struct Query(Sender<Vec<Box<Snapshot>>>);
impl SnapshotQueryInput for Query {
    type Time = u64;
    type Response = Sender<Vec<Box<Snapshot>>>;
    fn into_parts(self) -> (u64, Vec<u128>, Self::Response) {
        (100, vec![7], self.0)
    }
}
struct EventQuery(Sender<Vec<Event>>);
impl EventQueryInput for EventQuery {
    type Time = u64;
    type Response = Sender<Vec<Event>>;
    fn into_parts(self) -> (u128, u64, u64, Self::Response) {
        (7, 0, 100, self.0)
    }
}
#[derive(Clone)]
struct Listener;
impl SnapshotListener<u64> for Listener {
    fn registered(&self, _: u64, _: Vec<u128>) -> bool {
        true
    }
    fn replayed(&self, _: u64, _: Vec<u128>) -> bool {
        true
    }
}
struct Listen;
impl SnapshotListenInput for Listen {
    type Time = u64;
    type Listener = Listener;
    fn into_parts(self) -> (u64, Vec<u128>, Listener) {
        (100, vec![7], Listener)
    }
}
struct Advance(u64, Sender<()>);
impl AdvanceInput for Advance {
    type Time = u64;
    type Completion = Sender<()>;
    fn into_parts(self) -> (u64, Sender<()>) {
        (self.0, self.1)
    }
}
enum Message {
    Apply(ApplyBatch<Event, Sender<Vec<u128>>>),
    Query(Query),
    Events(EventQuery),
    Advance(Advance),
    Prune(Advance),
    Fence(u64, usize),
}
impl WorkInput for Message {
    type Apply = ApplyBatch<Event, Sender<Vec<u128>>>;
    type SnapshotQuery = Query;
    type EventQuery = EventQuery;
    type SnapshotListen = Listen;
    type Advance = Advance;
    fn into_kind(self) -> WorkInputKind<ApplyBatch<Event, Sender<Vec<u128>>>, Query, EventQuery, Listen, Advance> {
        match self {
            Self::Apply(batch) => WorkInputKind::Apply(batch),
            Self::Query(query) => WorkInputKind::SnapshotQuery(query),
            Self::Events(query) => WorkInputKind::EventQuery(query),
            Self::Advance(advance) => WorkInputKind::Advance(advance),
            Self::Prune(advance) => WorkInputKind::Prune(advance),
            Self::Fence(round, router) => WorkInputKind::Fence { round, router },
        }
    }
}
fn config() -> WorkerConfig {
    WorkerConfig {
        maximum_dirty_age: Duration::ZERO,
        replays_per_receive: 0,
        deadline_compaction_minimum: 0,
        deadline_compaction_multiplier: 0,
    }
}
fn apply(input: &Sender<Message>, values: &[(u128, u128, u64)]) -> Receiver<Vec<u128>> {
    let (completion, done) = unbounded();
    let inputs = values.iter().map(|&(snapshot_id, id, time)| RoutedInput { snapshot_id, input: Event { id, time } }).collect();
    input.send(Message::Apply(ApplyBatch { inputs, completion })).unwrap();
    done
}
fn advance(input: &Sender<Message>, time: u64) {
    input.send(Message::Advance(Advance(time, unbounded().0))).unwrap();
}
fn receive<T>(receiver: &Receiver<T>) -> T {
    receiver.recv_timeout(Duration::from_secs(5)).unwrap()
}
fn spawn(
    input: Receiver<Message>,
    steps: Sender<Snapshot>,
    gate: Option<Receiver<()>>,
    registrations: Receiver<Sender<bool>>,
    coordination: Option<Coordination<u64>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || work_messages::<_, TestStore>(input, config(), (), 0, Context { steps, gate }, registrations, coordination))
}

#[test]
fn timestamp_order_across_snapshots_uses_snapshot_id_for_ties() {
    let expected = vec![(20, 9, vec![3]), (30, 7, vec![2, 5, 4]), (30, 9, vec![3, 1])];

    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    apply(&input, &[(9, 1, 30), (7, 2, 20), (9, 3, 10), (7, 4, 30), (7, 5, 20)]);
    advance(&input, 30);
    drop(input);
    spawn(receiver, steps, None, crossbeam_channel::never(), None).join().unwrap();
    let actual = observed.try_iter().map(|s| (s.time, s.id, s.ids)).collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn equal_time_snapshots_merge_into_the_next_bucket() {
    let expected = vec![(1, 20), (2, 20), (3, 40), (1, 40), (2, 40)];

    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    apply(&input, &[(1, 1, 10), (2, 2, 10), (3, 3, 20), (1, 4, 40), (2, 5, 40)]);
    advance(&input, 40);
    drop(input);

    spawn(receiver, steps, None, crossbeam_channel::never(), None).join().unwrap();
    let actual = observed.try_iter().map(|snapshot| (snapshot.id, snapshot.time)).collect::<Vec<_>>();

    assert_eq!(actual, expected);
}

#[test]
fn insertion_at_completed_boundary_reopens_that_timestamp() {
    let expected = vec![(20, vec![1]), (20, vec![1, 3]), (30, vec![1, 3, 2])];

    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    let (release, gate) = unbounded();
    apply(&input, &[(7, 1, 20), (7, 2, 30)]);
    advance(&input, 20);
    let worker = spawn(receiver, steps, Some(gate), crossbeam_channel::never(), None);

    let first = receive(&observed);
    apply(&input, &[(7, 3, 20)]);
    release.send(()).unwrap();
    let second = receive(&observed);
    advance(&input, 30);
    release.send(()).unwrap();
    let third = receive(&observed);
    drop(input);
    release.send(()).unwrap();
    worker.join().unwrap();
    let actual = [first, second, third].into_iter().map(|snapshot| (snapshot.time, snapshot.ids)).collect::<Vec<_>>();

    assert_eq!(actual, expected);
    assert!(observed.is_empty());
}

#[test]
fn query_and_late_input_are_served_between_complete_buckets() {
    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    let (release, gate) = unbounded();
    let (register, registrations) = unbounded();
    let (activity, states) = unbounded();
    register.send(activity).unwrap();
    let worker = spawn(receiver, steps, Some(gate), registrations, None);
    assert!(!receive(&states));
    apply(&input, &[(7, 1, 10), (7, 3, 20), (7, 4, 30), (9, 5, 20)]);
    advance(&input, 30);
    assert_eq!(receive(&observed).ids, vec![1, 3]);
    while states.try_recv().is_ok() {}
    assert!(states.is_empty(), "pending computation must stay working between steps");
    let done = apply(&input, &[(7, 2, 10)]);
    let (response, snapshots) = unbounded();
    input.send(Message::Query(Query(response))).unwrap();
    release.send(()).unwrap();
    assert_eq!(receive(&snapshots)[0].ids, vec![1, 2, 3, 4]);
    assert_eq!(done.recv_timeout(Duration::from_secs(5)), Err(crossbeam_channel::RecvTimeoutError::Disconnected));
    let corrected = receive(&observed);
    assert_eq!((corrected.time, corrected.ids), (20, vec![1, 2, 3]));
    release.send(()).unwrap();
    assert_eq!(receive(&observed).ids, vec![5]);
    release.send(()).unwrap();
    assert_eq!(receive(&observed).ids, vec![1, 2, 3, 4]);
    release.send(()).unwrap();
    drop(input);
    worker.join().unwrap();
}

#[test]
fn fences_report_between_callbacks_without_waiting_for_idle() {
    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    let (release, gate) = unbounded();
    let (reports, minima) = unbounded();
    apply(&input, &[(7, 1, 10), (7, 2, 20)]);
    advance(&input, 10);
    let coordination = Coordination {
        router_count: 1,
        report: Box::new(move |round, minimum| reports.send((round, minimum)).unwrap()),
        pruned: Box::new(|_| {}),
    };
    let worker = spawn(receiver, steps, Some(gate), crossbeam_channel::never(), Some(coordination));
    assert_eq!(receive(&observed).time, 10);
    input.send(Message::Fence(1, 0)).unwrap();
    assert!(minima.is_empty());
    release.send(()).unwrap();
    assert_eq!(receive(&minima), (1, Some(10)));
    advance(&input, 20);
    assert_eq!(receive(&observed).time, 20);
    input.send(Message::Fence(2, 0)).unwrap();
    release.send(()).unwrap();
    assert_eq!(receive(&minima), (2, None));
    drop(input);
    worker.join().unwrap();
}

#[test]
fn pruning_pending_history_is_an_invariant_violation() {
    let (input, receiver) = unbounded();
    let (steps, _observed) = unbounded();
    apply(&input, &[(7, 1, 10)]);
    input.send(Message::Prune(Advance(20, unbounded().0))).unwrap();
    drop(input);
    let result = spawn(receiver, steps, None, crossbeam_channel::never(), None).join();
    let panic = result.expect_err("unsafe prune must fail");
    assert_eq!(panic.downcast_ref::<&str>(), Some(&"prune would discard pending replay"));
}

#[test]
fn target_bounds_replay_future_only_is_idle_and_advance_does_not_prune() {
    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    let (register, registrations) = unbounded();
    let (activity, states) = unbounded();
    register.send(activity).unwrap();
    let worker = spawn(receiver, steps, None, registrations, None);
    assert!(!receive(&states));
    let done = apply(&input, &[(7, 1, 10), (7, 2, 20), (7, 3, 30)]);
    assert!(receive(&states));
    assert!(!receive(&states));
    assert!(observed.is_empty());
    assert_eq!(done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
    advance(&input, 20);
    assert!(receive(&states));
    assert!(!receive(&states));
    assert_eq!(observed.try_iter().map(|s| s.time).collect::<Vec<_>>(), vec![20]);
    advance(&input, 10);
    let (response, events) = unbounded();
    input.send(Message::Events(EventQuery(response))).unwrap();
    assert_eq!(receive(&events).iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert!(observed.is_empty());
    drop(input);
    worker.join().unwrap();
}

#[test]
fn fences_wait_for_distinct_routers_and_report_pending_replay_only_once() {
    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    let (reports, minima) = unbounded();
    apply(&input, &[(7, 1, 10)]);
    input.send(Message::Fence(1, 0)).unwrap();
    input.send(Message::Fence(1, 0)).unwrap();
    let (response, queries) = unbounded();
    input.send(Message::Query(Query(response))).unwrap();
    let coordination = Coordination {
        router_count: 2,
        report: Box::new(move |round, minimum| reports.send((round, minimum)).unwrap()),
        pruned: Box::new(|_| {}),
    };
    let worker = spawn(receiver, steps, None, crossbeam_channel::never(), Some(coordination));
    receive(&queries);
    assert!(minima.is_empty());
    apply(&input, &[(7, 2, 5)]);
    input.send(Message::Fence(1, 1)).unwrap();
    assert_eq!(receive(&minima), (1, Some(5)));
    input.send(Message::Fence(1, 1)).unwrap();
    input.send(Message::Fence(0, 0)).unwrap();
    input.send(Message::Fence(2, 0)).unwrap();
    input.send(Message::Fence(2, 1)).unwrap();
    assert_eq!(receive(&minima), (2, Some(5)));
    drop(input);
    worker.join().unwrap();
    assert!(minima.is_empty());
    assert!(observed.is_empty());
}

#[test]
fn explicit_prune_retains_boundary_and_sets_admission_horizon_for_new_histories() {
    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    let (register, registrations) = unbounded();
    let (activity, states) = unbounded();
    register.send(activity).unwrap();
    let worker = spawn(receiver, steps, None, registrations, None);
    assert!(!receive(&states));
    apply(&input, &[(7, 1, 10), (7, 2, 20), (7, 3, 30)]);
    advance(&input, 20);
    // Advance may be received in the same or the next activity cycle.
    assert_eq!(receive(&observed).time, 20);
    let (completion, pruned) = unbounded();
    input.send(Message::Prune(Advance(20, completion))).unwrap();
    assert_eq!(pruned.recv_timeout(Duration::from_secs(5)), Err(crossbeam_channel::RecvTimeoutError::Disconnected));
    let (response, events) = unbounded();
    input.send(Message::Events(EventQuery(response))).unwrap();
    assert_eq!(receive(&events).iter().map(|e| e.id).collect::<Vec<_>>(), vec![2, 3]);
    assert_eq!(receive(&apply(&input, &[(9, 4, 19)])), vec![4]);
    let (response, snapshots) = unbounded();
    input.send(Message::Query(Query(response))).unwrap();
    assert_eq!(receive(&snapshots)[0].ids, vec![1, 2, 3]);
    drop(input);
    worker.join().unwrap();
}

#[test]
fn queued_pruning_enforces_admission_on_existing_stores() {
    let horizon = 20;
    let expected_rejections = vec![2];
    let expected_events = vec![1, 3];

    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    apply(&input, &[(7, 1, horizon)]);
    input.send(Message::Prune(Advance(horizon, unbounded().0))).unwrap();
    let rejected = apply(&input, &[(7, 2, horizon - 1)]);
    let accepted = apply(&input, &[(7, 3, horizon)]);
    let (response, events) = unbounded();
    input.send(Message::Events(EventQuery(response))).unwrap();
    drop(input);

    let worker = spawn(receiver, steps, None, crossbeam_channel::never(), None);
    worker.join().unwrap();
    let actual_rejections = receive(&rejected);
    let actual_events = receive(&events).into_iter().map(|event| event.id).collect::<Vec<_>>();
    let actual_completion = accepted.try_recv();

    assert_eq!(actual_rejections, expected_rejections);
    assert_eq!(actual_events, expected_events);
    assert_eq!(actual_completion, Err(crossbeam_channel::TryRecvError::Disconnected));
    assert!(observed.is_empty());
}

#[test]
fn duplicate_insertion_does_not_reschedule_processed_history() {
    let expected_times = vec![20];

    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    let (release, gate) = unbounded();
    apply(&input, &[(7, 1, 10), (7, 2, 20)]);
    advance(&input, 20);
    let worker = spawn(receiver, steps, Some(gate), crossbeam_channel::never(), None);
    let first = receive(&observed);
    let done = apply(&input, &[(7, 1, 10)]);
    release.send(()).unwrap();
    drop(input);

    worker.join().unwrap();
    let actual_times = vec![first.time];
    let actual_completion = done.try_recv();

    assert_eq!(actual_times, expected_times);
    assert!(observed.is_empty());
    assert_eq!(actual_completion, Err(crossbeam_channel::TryRecvError::Disconnected));
}

#[test]
fn happy() {
    let rounds = 50;
    let expected_ids = (1..=rounds as u128).collect::<Vec<_>>();
    let expected_event_responses = vec![Err(crossbeam_channel::RecvTimeoutError::Disconnected); rounds as usize];
    let expected_application_count = rounds;

    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    let worker = spawn(receiver, steps, None, crossbeam_channel::never(), None);
    let mut actual_application_count = 0;
    let mut actual_ids = Vec::new();
    let mut actual_event_responses = Vec::new();

    for time in 1..=rounds {
        apply(&input, &[(7, time as u128, time)]);
        advance(&input, time);
        let snapshot = receive(&observed);
        actual_application_count += 1;
        actual_ids = snapshot.ids;
        let (response, queries) = unbounded();
        input.send(Message::Query(Query(response))).unwrap();
        assert_eq!(receive(&queries)[0].ids, actual_ids);
        let (completion, pruned) = unbounded();
        input.send(Message::Prune(Advance(time + 1, completion))).unwrap();
        assert_eq!(pruned.recv_timeout(Duration::from_secs(5)), Err(crossbeam_channel::RecvTimeoutError::Disconnected));
        let (response, events) = unbounded();
        input.send(Message::Events(EventQuery(response))).unwrap();
        actual_event_responses.push(events.recv_timeout(Duration::from_secs(5)));
    }
    drop(input);
    worker.join().unwrap();

    assert_eq!(actual_application_count, expected_application_count);
    assert_eq!(actual_ids, expected_ids);
    assert_eq!(actual_event_responses, expected_event_responses);
    assert!(observed.is_empty());
}
