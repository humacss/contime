use std::thread::JoinHandle;
use std::time::Duration;

use contime_checkpoints as replay;
use contime_events::{EventHistory, Insert};
use contime_worker::{
    work_messages, AdvanceInput, ApplyBatch, Checkpoints, Coordination, EventInsert, EventQueryInput, Events, IncrementalCheckpoints,
    QueryCheckpoints, QueryEvents, RoutedInput, SnapshotListenInput, SnapshotListener, SnapshotQueryInput, WorkInput, WorkInputKind,
    WorkerConfig,
};
use crossbeam_channel::{unbounded, Receiver, Sender};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Event {
    id: u128,
    time: u64,
}

impl contime_events::Event for Event {
    type Time = u64;
    fn event_id(&self) -> u128 {
        self.id
    }
    fn time(&self) -> u64 {
        self.time
    }
}

struct History(EventHistory<Event>);
impl Events<Event> for History {
    type Config = ();
    type Rejection = u128;
    type Time = u64;
    fn create(_: u128, _: &(), horizon: &u64) -> Self {
        Self(EventHistory::with_horizon(*horizon))
    }
    fn insert(&mut self, event: Event) -> EventInsert<u128> {
        let id = event.id;
        let outcome = self.0.insert(event);
        EventInsert { changed: outcome == Insert::Inserted, rejections: if outcome == Insert::BeforeHorizon { vec![id] } else { vec![] } }
    }
    fn dirty_time(&self) -> &u64 {
        self.0.dirty_time()
    }
    fn prune_before(&mut self, horizon: &u64) {
        self.0.prune_before(horizon);
    }
}
impl QueryEvents<Event> for History {
    type Time = u64;
    fn clone_between(&self, from: &u64, to: &u64) -> Vec<Event> {
        self.0.clone_between(from, to)
    }
}
impl replay::Events for History {
    type Time = u64;
    type Event = Event;
    type Iter<'a> = Box<dyn Iterator<Item = replay::EventRef<'a, u64, Event>> + 'a>;
    fn dirty_time(&self) -> &u64 {
        self.0.dirty_time()
    }
    fn iter_after(&self, boundary: Option<&replay::CheckpointKey<u64>>) -> Self::Iter<'_> {
        let boundary = boundary.cloned();
        Box::new(
            self.0
                .iter()
                .filter(move |(key, _)| boundary.as_ref().is_none_or(|b| (key.time, key.event_id) > (b.time, b.event_id)))
                .map(|(key, event)| replay::EventRef { time: &key.time, event_id: key.event_id, event }),
        )
    }
    fn acknowledge_replay(&mut self) {
        self.0.mark_replayed();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    id: u128,
    time: u64,
    ids: Vec<u128>,
}
impl replay::Snapshot for Snapshot {
    type Time = u64;
    fn set_time(&mut self, time: u64) {
        self.time = time;
    }
}
impl replay::ApplyEvents<Event> for Snapshot {
    fn create(id: u128, _: &Event) -> Self {
        Self { id, time: 0, ids: vec![] }
    }
    fn apply_events(&mut self, batch: replay::ApplyBatch<'_, u64, Event>) {
        self.ids.extend(batch.events.iter().map(|event| event.id));
    }
}
struct Context {
    steps: Sender<Snapshot>,
    gate: Option<Receiver<()>>,
}
impl replay::ApplyWrapper<Snapshot, Event> for Context {
    fn replay_event_batch(&mut self, batch: replay::EventBatch<'_, u64, Event>, inner: &mut replay::ApplyInner<'_, Snapshot>) {
        inner.apply_event_batch(batch);
        self.steps.send(inner.snapshot().clone()).unwrap();
        if let Some(gate) = &self.gate {
            gate.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }
}
struct Checkpoint(replay::CheckpointStore<Snapshot>);
impl Checkpoints<History> for Checkpoint {
    type Config = ();
    type Context = Context;
    type Time = u64;
    fn create(id: u128, _: &()) -> Self {
        Self(replay::CheckpointStore::new(id, replay::CheckpointConfig { interval: 1 }))
    }
    fn update(&mut self, history: &mut History, context: &mut Context) -> u64 {
        let time = *history.0.dirty_time();
        replay::replay(&mut self.0, history, context);
        time
    }
    fn advance_before(&mut self, history: &History, context: &mut Context, horizon: &u64) {
        replay::advance_before(&mut self.0, history, context, horizon);
    }
}
impl IncrementalCheckpoints<History> for Checkpoint {
    fn invalidate(&mut self, history: &mut History) {
        self.0.invalidate_from(history.0.dirty_time());
        history.0.mark_replayed();
    }
    fn next_time(&self, history: &History) -> Option<u64> {
        self.0.next_replay_time(history)
    }
    fn step(&mut self, history: &History, context: &mut Context, target: &u64) {
        replay::replay_next(&mut self.0, history, context, target);
    }
}
impl QueryCheckpoints<History> for Checkpoint {
    type Context = Context;
    type Time = u64;
    type Snapshot = Snapshot;
    fn query_at(&self, history: &History, context: &mut Context, time: u64) -> Option<Box<Snapshot>> {
        replay::query_at(&self.0, history, context, time)
    }
}

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
    std::thread::spawn(move || {
        work_messages::<_, History, Checkpoint>(input, config(), (), 0, (), Context { steps, gate }, registrations, coordination)
    })
}

#[test]
fn timestamp_order_across_snapshots_uses_snapshot_id_for_ties() {
    let (input, receiver) = unbounded();
    let (steps, observed) = unbounded();
    apply(&input, &[(9, 1, 30), (7, 2, 20), (9, 3, 10), (7, 4, 30), (7, 5, 20)]);
    advance(&input, 30);
    drop(input);
    spawn(receiver, steps, None, crossbeam_channel::never(), None).join().unwrap();
    let actual = observed.try_iter().map(|s| (s.time, s.id, s.ids)).collect::<Vec<_>>();
    assert_eq!(actual, vec![(10, 9, vec![3]), (20, 7, vec![2, 5]), (30, 7, vec![2, 5, 4]), (30, 9, vec![3, 1])]);
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
    apply(&input, &[(7, 1, 10), (7, 3, 20), (7, 4, 30)]);
    advance(&input, 30);
    assert_eq!(receive(&observed).ids, vec![1]);
    while states.try_recv().is_ok() {}
    release.send(()).unwrap();
    assert_eq!(receive(&observed).ids, vec![1, 3]);
    assert!(states.is_empty(), "pending computation must stay working between steps");
    let done = apply(&input, &[(7, 2, 10)]);
    let (response, snapshots) = unbounded();
    input.send(Message::Query(Query(response))).unwrap();
    release.send(()).unwrap();
    assert_eq!(receive(&snapshots)[0].ids, vec![1, 2, 3, 4]);
    assert_eq!(done.recv_timeout(Duration::from_secs(5)), Err(crossbeam_channel::RecvTimeoutError::Disconnected));
    let corrected = receive(&observed);
    assert_eq!((corrected.time, corrected.ids), (10, vec![1, 2]));
    release.send(()).unwrap();
    assert_eq!(receive(&observed).ids, vec![1, 2, 3]);
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
    advance(&input, 20);
    let coordination = Coordination { router_count: 1, report: Box::new(move |round, minimum| reports.send((round, minimum)).unwrap()) };
    let worker = spawn(receiver, steps, Some(gate), crossbeam_channel::never(), Some(coordination));
    assert_eq!(receive(&observed).time, 10);
    input.send(Message::Fence(1, 0)).unwrap();
    assert!(minima.is_empty());
    release.send(()).unwrap();
    assert_eq!(receive(&minima), (1, Some(20)));
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
    assert_eq!(observed.try_iter().map(|s| s.time).collect::<Vec<_>>(), vec![10, 20]);
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
    let coordination = Coordination { router_count: 2, report: Box::new(move |round, minimum| reports.send((round, minimum)).unwrap()) };
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
    assert_eq!(receive(&observed).time, 10);
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
