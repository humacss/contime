use std::thread::JoinHandle;
use std::time::Duration;

use contime_worker::{
    work_messages, AdvanceInput, ApplyBatch, Coordination, EventInsert, EventQueryInput, RoutedInput, SnapshotListenInput,
    SnapshotListener, SnapshotQueryInput, SnapshotStore, WorkInput, WorkInputKind, WorkerStatus,
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
    fn earliest_replay_time(&self) -> Option<u64> {
        self.events.iter().find(|event| self.processed.is_none_or(|time| event.time > time)).map(|event| event.time)
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
    Packet(Vec<ApplyBatch<Event, Sender<Vec<u128>>>>),
    Status(usize, Sender<WorkerStatus<u64>>),
    Listen(Listen),
    Query(Query),
    Events(EventQuery),
    Advance(Advance),
    Prune(Advance),
    Fence(u64, usize),
    Resolve(u64, Advance),
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
            Self::Packet(batches) => WorkInputKind::ApplyPacket(batches),
            Self::Status(worker_id, response) => WorkInputKind::Status { worker_id, response },
            Self::Listen(listen) => WorkInputKind::SnapshotListen(listen),
            Self::Query(query) => WorkInputKind::SnapshotQuery(query),
            Self::Events(query) => WorkInputKind::EventQuery(query),
            Self::Advance(advance) => WorkInputKind::Advance(advance),
            Self::Prune(advance) => WorkInputKind::Prune(advance),
            Self::Fence(round, router) => WorkInputKind::Fence { round, router },
            Self::Resolve(round, prune) => WorkInputKind::Resolve { round, prune },
        }
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
    std::thread::spawn(move || work_messages::<_, TestStore>(input, (), Context { steps, gate }, registrations, coordination))
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
    assert_eq!(receive(&minima), (1, Some(20)));
    input.send(Message::Resolve(1, Advance(0, unbounded().0))).unwrap();
    advance(&input, 20);
    assert_eq!(receive(&observed).time, 20);
    input.send(Message::Fence(2, 0)).unwrap();
    release.send(()).unwrap();
    assert_eq!(receive(&minima), (2, None));
    drop(input);
    worker.join().unwrap();
}

#[test]
fn consecutive_rounds_allow_processing_between_reports() {
    let expected_first = (1, Some(10));
    let expected_second = (2, None);

    let (input, messages) = unbounded();
    let registrations = crossbeam_channel::never();
    let (steps, observed) = unbounded();
    let (reports, minima) = unbounded();
    let mut worker = contime_worker::MessageWorker::<Message, TestStore>::new(
        &messages,
        &registrations,
        (),
        Context { steps, gate: None },
        Some(Coordination {
            router_count: 1,
            report: Box::new(move |round, minimum| reports.send((round, minimum)).unwrap()),
            pruned: Box::new(|_| {}),
        }),
    );
    apply(&input, &[(7, 1, 10)]);
    worker.handle(messages.try_recv().unwrap());
    worker.handle(Message::Advance(Advance(10, unbounded().0)));
    worker.handle(Message::Fence(1, 0));
    let first = minima.try_recv().unwrap();

    worker.handle(Message::Resolve(1, Advance(0, unbounded().0)));
    worker.handle(Message::Fence(2, 0));
    assert!(minima.is_empty(), "the next round must allow a processing step first");
    let processed = worker.step();
    let second = minima.try_recv().unwrap();

    assert_eq!(first, expected_first);
    assert!(processed);
    assert_eq!(observed.try_recv().unwrap().time, 10);
    assert_eq!(second, expected_second);
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

#[test]
fn sparse_histories_never_replay_an_event_free_gap() {
    let (input, messages) = unbounded();
    let (steps, observed) = unbounded();
    let registrations = crossbeam_channel::never();
    let mut worker =
        contime_worker::MessageWorker::<Message, TestStore>::new(&messages, &registrations, (), Context { steps, gate: None }, None);
    let done = apply(&input, &[(7, 1, 1), (9, 2, 2), (11, 3, 3), (7, 4, 4), (9, 5, 5), (11, 6, 6)]);
    worker.handle(messages.try_recv().unwrap());
    worker.handle(Message::Advance(Advance(6, unbounded().0)));
    let mut counts = std::collections::BTreeMap::new();
    while worker.step() {
        let snapshot = observed.try_recv().unwrap();
        let previous = counts.insert(snapshot.id, snapshot.ids.len()).unwrap_or(0);
        assert!(snapshot.ids.len() > previous, "snapshot {} replayed an event-free gap", snapshot.id);
    }
    assert_eq!(counts, [(7, 2), (9, 2), (11, 2)].into());
    assert_eq!(done.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
    assert!(observed.is_empty());
}

#[test]
fn sparse_future_replay_waits_for_its_inclusive_target() {
    let (input, messages) = unbounded();
    let (steps, observed) = unbounded();
    let registrations = crossbeam_channel::never();
    let mut worker =
        contime_worker::MessageWorker::<Message, TestStore>::new(&messages, &registrations, (), Context { steps, gate: None }, None);
    apply(&input, &[(7, 1, 10), (7, 4, 100), (9, 2, 20), (11, 3, 30)]);
    worker.handle(messages.try_recv().unwrap());
    worker.handle(Message::Advance(Advance(30, unbounded().0)));
    while worker.step() {}
    let first = observed.try_iter().map(|s| (s.id, s.ids)).collect::<Vec<_>>();
    assert_eq!(first, [(7, vec![1]), (9, vec![2]), (11, vec![3])]);
    worker.handle(Message::Advance(Advance(99, unbounded().0)));
    assert!(!worker.step());
    assert!(observed.is_empty());
    worker.handle(Message::Advance(Advance(100, unbounded().0)));
    assert!(worker.step());
    let final_state = observed.try_recv().unwrap();
    assert_eq!((final_state.id, final_state.ids), (7, vec![1, 4]));
    assert!(!worker.step());
    assert!(observed.is_empty());
}

#[test]
fn completed_history_reopens_for_late_same_time_input_but_not_a_duplicate() {
    let (input, messages) = unbounded();
    let (steps, observed) = unbounded();
    let registrations = crossbeam_channel::never();
    let mut worker =
        contime_worker::MessageWorker::<Message, TestStore>::new(&messages, &registrations, (), Context { steps, gate: None }, None);
    apply(&input, &[(7, 2, 0), (7, 3, 10), (9, 4, 5)]);
    worker.handle(messages.try_recv().unwrap());
    worker.handle(Message::Advance(Advance(10, unbounded().0)));
    while worker.step() {}
    observed.try_iter().for_each(drop);
    apply(&input, &[(7, 2, 0)]);
    worker.handle(messages.try_recv().unwrap());
    assert!(!worker.step());
    assert!(observed.is_empty());
    apply(&input, &[(7, 1, 0)]);
    worker.handle(messages.try_recv().unwrap());
    assert!(worker.step());
    let corrected = observed.try_recv().unwrap();
    assert_eq!((corrected.id, corrected.ids), (7, vec![1, 2, 3]));
    assert!(!worker.step());
    assert!(observed.is_empty());
}

#[test]
fn status_distinguishes_metadata_stores_future_work_and_completed_replay() {
    let (input, messages) = unbounded();
    let (steps, observed) = unbounded();
    let registrations = crossbeam_channel::never();
    let mut worker =
        contime_worker::MessageWorker::<Message, TestStore>::new(&messages, &registrations, (), Context { steps, gate: None }, None);
    let empty = WorkerStatus {
        worker_id: 4,
        history_stores: 0,
        metadata_only: 0,
        pending_histories: 0,
        due_histories: 0,
        processing_target: 0,
        admission_horizon: 0,
        pending_prune_passes: 0,
        awaiting_resolution: None,
        counters: cfg!(feature = "monitor").then(Default::default),
    };
    assert_eq!(worker.status(4), empty);
    worker.handle(Message::Listen(Listen));
    assert_eq!(worker.status(4), WorkerStatus { metadata_only: 1, ..empty.clone() });
    apply(&input, &[(7, 1, 10), (9, 2, 50), (11, 3, 20)]);
    worker.handle(messages.try_recv().unwrap());
    worker.handle(Message::Advance(Advance(20, unbounded().0)));
    let pending = WorkerStatus {
        counters: cfg!(feature = "monitor").then_some(contime_worker::WorkerCounters { inserted_applications: 3, ..Default::default() }),
        history_stores: 3,
        pending_histories: 3,
        due_histories: 2,
        processing_target: 20,
        ..empty
    };
    assert_eq!(worker.status(4), pending);
    let (response, result) = unbounded();
    worker.handle(Message::Status(4, response));
    assert_eq!(result.try_recv().unwrap(), pending);
    assert_eq!(worker.status(4), pending, "a status request does not change scheduling");
    assert!(observed.is_empty(), "status must not execute store replay or reconstruction");
    while worker.step() {}
    assert_eq!(observed.try_iter().count(), 2);
    assert_eq!(
        worker.status(4),
        WorkerStatus {
            counters: cfg!(feature = "monitor").then_some(contime_worker::WorkerCounters {
                inserted_applications: 3,
                replay_applications: None,
                saturated: false
            }),
            pending_histories: 1,
            due_histories: 0,
            ..pending
        }
    );
}

#[test]
fn status_reports_pruning_and_coordination_without_servicing_them() {
    let (input, messages) = unbounded();
    let (steps, observed) = unbounded();
    let (reports, report_rx) = unbounded();
    let (pruned, pruned_rx) = unbounded();
    let registrations = crossbeam_channel::never();
    let coordination = Coordination {
        router_count: 1,
        report: Box::new(move |round, earliest| reports.send((round, earliest)).unwrap()),
        pruned: Box::new(move |horizon| pruned.send(horizon).unwrap()),
    };
    let mut worker = contime_worker::MessageWorker::<Message, TestStore>::new(
        &messages,
        &registrations,
        (),
        Context { steps, gate: None },
        Some(coordination),
    );
    apply(&input, &[(7, 1, 10), (9, 2, 20)]);
    worker.handle(messages.try_recv().unwrap());
    worker.handle(Message::Advance(Advance(20, unbounded().0)));
    worker.handle(Message::Fence(6, 0));
    assert_eq!(report_rx.try_recv().unwrap(), (6, Some(10)));
    let blocked = worker.status(1);
    assert_eq!((blocked.pending_histories, blocked.due_histories, blocked.awaiting_resolution), (2, 2, Some(6)));
    let (response, result) = unbounded();
    worker.handle(Message::Status(1, response));
    assert_eq!(result.try_recv().unwrap(), blocked);
    assert!(!worker.step());
    assert!(observed.is_empty());
    assert!(report_rx.is_empty(), "status must not publish a coordination report");
    worker.handle(Message::Resolve(6, Advance(5, unbounded().0)));
    worker.handle(Message::Prune(Advance(6, unbounded().0)));
    let pruning = worker.status(1);
    assert_eq!((pruning.admission_horizon, pruning.pending_prune_passes, pruning.awaiting_resolution), (6, 2, None));
    let (response, result) = unbounded();
    worker.handle(Message::Status(1, response));
    assert_eq!(result.try_recv().unwrap(), pruning);
    assert!(pruned_rx.is_empty(), "status must not service a prune pass");
    assert_eq!(worker.status(1), pruning);
    while worker.prune_step() {}
    assert_eq!(pruned_rx.try_iter().collect::<Vec<_>>(), [5, 6]);
    assert_eq!(worker.status(1).pending_prune_passes, 0);
}

#[test]
fn full_and_disconnected_status_receivers_do_not_block_or_change_state() {
    let (steps, _) = unbounded();
    let mut worker = contime_worker::MessageWorker::<Message, TestStore>::new_inline((), Context { steps, gate: None });
    let initial = worker.status(0);
    let (response, result) = crossbeam_channel::bounded(1);
    response.send(initial.clone()).unwrap();
    worker.handle(Message::Status(0, response));
    assert_eq!(result.try_recv().unwrap(), initial);
    assert_eq!(result.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected));
    let (response, result) = crossbeam_channel::bounded(1);
    drop(result);
    worker.handle(Message::Status(0, response));
    assert_eq!(worker.status(0), initial);
}

#[test]
fn threaded_worker_services_status_without_advancing_future_inputs() {
    let (input, messages) = unbounded();
    let (steps, observed) = unbounded();
    apply(&input, &[(7, 1, 10)]);
    let (response, result) = unbounded();
    input.send(Message::Status(3, response)).unwrap();
    drop(input);
    let worker = spawn(messages, steps, None, crossbeam_channel::never(), None);
    let status = receive(&result);
    worker.join().unwrap();
    assert_eq!((status.worker_id, status.history_stores, status.pending_histories, status.due_histories), (3, 1, 1, 0));
    assert!(observed.is_empty());
}

#[test]
fn status_never_calls_storage_read_replay_or_retention_hooks() {
    // This adapter permits insertion but makes every storage observation and
    // execution hook fail, including the scheduler's earliest-time lookup.
    struct InsertionOnly(TestStore);
    impl SnapshotStore<Event> for InsertionOnly {
        type Config = ();
        type Context = Context;
        type Time = u64;
        type Snapshot = Snapshot;
        type Rejection = u128;

        fn create(id: u128, config: &(), horizon: &u64) -> Self {
            Self(TestStore::create(id, config, horizon))
        }
        fn insert(&mut self, event: Event, horizon: &u64) -> EventInsert<u128> {
            self.0.insert(event, horizon)
        }
        fn event_time(event: &Event) -> u64 {
            TestStore::event_time(event)
        }
        fn earliest_replay_time(&self) -> Option<u64> {
            panic!("status must not inspect storage replay state")
        }
        fn process_until(&mut self, _: &u64, _: &mut Context) {
            panic!("status must not replay events")
        }
        fn query(&mut self, _: u64, _: &mut Context) -> Option<Box<Snapshot>> {
            panic!("status must not reconstruct snapshots")
        }
        fn query_events(&self, _: &u64, _: &u64) -> Vec<Event> {
            panic!("status must not read events")
        }
        fn forward(&mut self, _: &u64, _: &mut Context) {
            panic!("status must not forward histories")
        }
        fn prune(&mut self) {
            panic!("status must not prune histories")
        }
    }
    let (steps, _) = unbounded();
    let mut worker = contime_worker::MessageWorker::<Message, InsertionOnly>::new_inline((), Context { steps, gate: None });
    worker.handle(Message::Apply(ApplyBatch {
        inputs: vec![RoutedInput { snapshot_id: 7, input: Event { id: 1, time: 10 } }],
        completion: unbounded().0,
    }));
    worker.handle(Message::Advance(Advance(20, unbounded().0)));
    worker.handle(Message::Prune(Advance(5, unbounded().0)));
    let before = worker.status(0);
    let (response, result) = unbounded();
    worker.handle(Message::Status(0, response));
    assert_eq!(result.try_recv().unwrap(), before);
    assert_eq!((before.history_stores, before.due_histories, before.pending_prune_passes), (1, 1, 1));
    assert_eq!(worker.status(0), before);
}

#[test]
fn counters_default_and_disabled_status_have_stable_shape() {
    let counters = contime_worker::WorkerCounters::default();
    let copied = counters;
    assert_eq!(copied.inserted_applications, 0);
    assert_eq!(copied.replay_applications, Some(0));
    assert!(!copied.saturated);
    let (steps, _observed) = unbounded();
    let worker = contime_worker::MessageWorker::<Message, TestStore>::new_inline((), Context { steps, gate: None });
    assert_eq!(worker.status(0).counters, cfg!(feature = "monitor").then_some(counters));
}

fn batch(values: &[(u128, u128, u64)]) -> ApplyBatch<Event, Sender<Vec<u128>>> {
    ApplyBatch {
        inputs: values.iter().map(|&(snapshot_id, id, time)| RoutedInput { snapshot_id, input: Event { id, time } }).collect(),
        completion: unbounded().0,
    }
}

#[test]
fn packet_insertion_counts_canonical_changes_and_excludes_duplicates_and_rejections() {
    let (steps, _observed) = unbounded();
    let mut worker = contime_worker::MessageWorker::<Message, TestStore>::new_inline((), Context { steps, gate: None });
    worker.handle(Message::Prune(Advance(10, unbounded().0)));
    while worker.prune_step() {}
    worker.handle(Message::Packet(vec![batch(&[(7, 1, 10), (7, 1, 10)]), batch(&[(9, 1, 10), (7, 2, 9)])]));
    assert_eq!(
        worker.status(0).counters,
        cfg!(feature = "monitor").then_some(contime_worker::WorkerCounters { inserted_applications: 2, ..Default::default() })
    );
}

#[cfg(feature = "monitor")]
#[test]
fn uncounted_custom_store_marks_replay_unknown_only_when_it_runs() {
    let (steps, observed) = unbounded();
    let mut worker = contime_worker::MessageWorker::<Message, TestStore>::new_inline((), Context { steps, gate: None });
    worker.handle(Message::Apply(batch(&[(7, 1, 10)])));
    assert_eq!(worker.status(0).counters.unwrap().replay_applications, Some(0));
    worker.handle(Message::Advance(Advance(10, unbounded().0)));
    assert!(worker.step());
    assert_eq!(observed.try_iter().count(), 1, "default counted method executes normal processing");
    assert_eq!(worker.status(0).counters.unwrap().replay_applications, None);
}

#[cfg(feature = "monitor")]
struct CountedStore(TestStore);

#[cfg(feature = "monitor")]
impl SnapshotStore<Event> for CountedStore {
    type Config = ();
    type Context = Context;
    type Time = u64;
    type Snapshot = Snapshot;
    type Rejection = u128;
    fn create(id: u128, config: &(), horizon: &u64) -> Self {
        Self(TestStore::create(id, config, horizon))
    }
    fn insert(&mut self, event: Event, horizon: &u64) -> EventInsert<u128> {
        self.0.insert(event, horizon)
    }
    fn event_time(event: &Event) -> u64 {
        event.time
    }
    fn earliest_replay_time(&self) -> Option<u64> {
        self.0.earliest_replay_time()
    }
    fn process_until(&mut self, time: &u64, context: &mut Context) {
        self.0.process_until(time, context);
    }
    fn process_until_counted(&mut self, time: &u64, context: &mut Context) -> Option<u64> {
        // This stub reconstructs from its origin on every processing call.
        let count = self.0.events.iter().filter(|event| event.time <= *time).count() as u64;
        self.process_until(time, context);
        Some(count)
    }
    fn query(&mut self, time: u64, context: &mut Context) -> Option<Box<Snapshot>> {
        self.0.query(time, context)
    }
    fn query_events(&self, from: &u64, to: &u64) -> Vec<Event> {
        self.0.query_events(from, to)
    }
    fn forward(&mut self, horizon: &u64, context: &mut Context) {
        self.0.forward(horizon, context);
    }
    fn prune(&mut self) {
        self.0.prune();
    }
}

#[cfg(feature = "monitor")]
#[test]
fn counted_replay_includes_late_reconstruction_and_excludes_queries_and_pruning() {
    let (steps, observed) = unbounded();
    let mut worker = contime_worker::MessageWorker::<Message, CountedStore>::new_inline((), Context { steps, gate: None });
    worker.handle(Message::Apply(batch(&[(7, 1, 10), (7, 2, 20)])));
    worker.handle(Message::Advance(Advance(20, unbounded().0)));
    assert!(worker.step());
    assert_eq!(worker.status(0).counters.unwrap().replay_applications, Some(2));
    worker.handle(Message::Apply(batch(&[(7, 3, 10), (7, 1, 10)])));
    assert!(worker.step());
    assert!(!worker.step());
    let before = worker.status(0).counters.unwrap();
    assert_eq!(before, contime_worker::WorkerCounters { inserted_applications: 3, replay_applications: Some(5), saturated: false });
    let (response, result) = unbounded();
    worker.handle(Message::Query(Query(response)));
    assert_eq!(result.try_recv().unwrap()[0].ids.len(), 3);
    let (response, result) = unbounded();
    worker.handle(Message::Events(EventQuery(response)));
    assert_eq!(result.try_recv().unwrap().len(), 3);
    worker.handle(Message::Prune(Advance(20, unbounded().0)));
    while worker.prune_step() {}
    assert_eq!(worker.status(0).counters, Some(before));
    assert_eq!(observed.try_iter().count(), 2);
}
