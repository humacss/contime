use contime_manual::{FrameBudget, ManualConTime, StopReason};
use std::time::Duration;
use std::{cell::Cell, num::NonZeroUsize, rc::Rc};

use contime_worker::{
    AdvanceInput, ApplyBatch, EventInsert, EventQueryInput, RoutedInput, SnapshotListenInput, SnapshotListener, SnapshotQueryInput,
    SnapshotStore, WorkInput, WorkInputKind,
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
    clock: Rc<Cell<Duration>>,
    cost: Duration,
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
        context.clock.set(context.clock.get() + context.cost);
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
#[derive(Clone)]
struct Advance(u64, Sender<()>);
impl AdvanceInput for Advance {
    type Time = u64;
    type Completion = Sender<()>;
    fn into_parts(self) -> (u64, Sender<()>) {
        (self.0, self.1)
    }
}
#[allow(dead_code)]
enum Message {
    Apply(ApplyBatch<Event, Sender<Vec<u128>>>),
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
            Self::Query(query) => WorkInputKind::SnapshotQuery(query),
            Self::Events(query) => WorkInputKind::EventQuery(query),
            Self::Advance(advance) => WorkInputKind::Advance(advance),
            Self::Prune(advance) => WorkInputKind::Prune(advance),
            Self::Fence(round, router) => WorkInputKind::Fence { round, router },
            Self::Resolve(round, prune) => WorkInputKind::Resolve { round, prune },
        }
    }
}

fn setup(cost: Duration) -> (ManualConTime<Message, TestStore>, Receiver<Snapshot>, Rc<Cell<Duration>>) {
    let (steps, observed) = unbounded();
    let clock = Rc::new(Cell::new(Duration::ZERO));
    (ManualConTime::new(NonZeroUsize::new(16).unwrap(), (), Context { steps, clock: clock.clone(), cost }), observed, clock)
}
fn put(driver: &mut ManualConTime<Message, TestStore>, message: Message) {
    assert!(driver.enqueue(message).is_ok());
}
fn inputs(driver: &mut ManualConTime<Message, TestStore>, events: &[(u128, u128, u64)]) -> Receiver<Vec<u128>> {
    let (sender, response) = unbounded();
    put(
        driver,
        Message::Apply(ApplyBatch {
            inputs: events.iter().map(|&(snapshot_id, id, time)| RoutedInput { snapshot_id, input: Event { id, time } }).collect(),
            completion: sender,
        }),
    );
    response
}
fn frame(driver: &mut ManualConTime<Message, TestStore>, steps: usize, clock: &Rc<Cell<Duration>>) -> contime_manual::FrameReport {
    driver.process_with_clock(FrameBudget { duration: Duration::from_millis(2), max_steps: steps }, || clock.get()).unwrap()
}
#[test]
fn inline_worker_runs_on_the_caller_with_non_send_context_and_complete_queries() {
    let (mut driver, _observed, clock) = setup(Duration::ZERO);
    let receipt = inputs(&mut driver, &[(7, 2, 2), (7, 1, 1)]);
    let (sender, answer) = unbounded();
    put(&mut driver, Message::Query(Query(sender)));
    assert_eq!(frame(&mut driver, 0, &clock).steps, 0);
    assert!(answer.is_empty());
    assert_eq!(frame(&mut driver, 1, &clock).steps, 1);
    assert!(receipt.is_empty());
    assert!(matches!(receipt.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected)));
    frame(&mut driver, 1, &clock);
    assert_eq!(answer.try_recv().unwrap()[0].ids, [1, 2]);
}
#[test]
fn replay_gets_a_turn_even_with_one_step_frames_and_continuous_messages() {
    let (mut driver, observed, clock) = setup(Duration::ZERO);
    inputs(&mut driver, &[(7, 1, 1)]);
    put(&mut driver, Message::Advance(Advance(10, unbounded().0)));
    frame(&mut driver, 1, &clock);
    frame(&mut driver, 1, &clock);
    for _ in 0..4 {
        put(&mut driver, Message::Query(Query(unbounded().0)));
        frame(&mut driver, 1, &clock);
    }
    assert_eq!(observed.try_recv().unwrap().ids, [1]);
}
#[test]
fn prune_waits_for_replay_and_finishes_before_later_admissions() {
    let (mut driver, observed, clock) = setup(Duration::ZERO);
    inputs(&mut driver, &[(7, 1, 1), (7, 2, 2)]);
    put(&mut driver, Message::Advance(Advance(10, unbounded().0)));
    let (done, completed) = unbounded();
    put(&mut driver, Message::Prune(Advance(5, done)));
    let expired = inputs(&mut driver, &[(7, 3, 1)]);
    for _ in 0..30 {
        if frame(&mut driver, 1, &clock).stopped == StopReason::NoReadyWork {
            break;
        }
    }
    assert_eq!(observed.try_recv().unwrap().ids, [1, 2]);
    assert!(matches!(completed.try_recv(), Err(crossbeam_channel::TryRecvError::Disconnected)));
    assert_eq!(expired.try_recv().unwrap(), [3]);
}
#[test]
fn prune_above_the_target_is_rejected_without_panicking_or_blocking_later_work() {
    let (mut driver, _observed, clock) = setup(Duration::ZERO);
    put(&mut driver, Message::Prune(Advance(10, unbounded().0)));
    assert!(driver.process_with_clock(FrameBudget { duration: Duration::from_secs(1), max_steps: 20 }, || clock.get()).is_err());
    inputs(&mut driver, &[(7, 1, 1)]);
    let (sender, answer) = unbounded();
    put(&mut driver, Message::Query(Query(sender)));
    frame(&mut driver, 10, &clock);
    assert_eq!(answer.try_recv().unwrap()[0].ids, [1]);
}
#[test]
fn whole_replay_calls_can_overrun_the_soft_budget() {
    let (mut driver, _observed, clock) = setup(Duration::from_millis(3));
    inputs(&mut driver, &[(7, 1, 1), (7, 2, 2), (7, 3, 3)]);
    put(&mut driver, Message::Advance(Advance(10, unbounded().0)));
    let report = frame(&mut driver, 100, &clock);
    assert!(report.overrun);
    assert_eq!(report.elapsed, Duration::from_millis(3));
}
#[test]
fn queue_capacity_and_zero_time_preserve_messages() {
    let (mut driver, _observed, clock) = setup(Duration::ZERO);
    for _ in 0..16 {
        put(&mut driver, Message::Advance(Advance(10, unbounded().0)));
    }
    assert!(driver.enqueue(Message::Advance(Advance(11, unbounded().0))).is_err());
    let report = driver.process_with_clock(FrameBudget { duration: Duration::ZERO, max_steps: 100 }, || clock.get()).unwrap();
    assert_eq!(report.steps, 0);
    assert_eq!(report.queued, 16);
}

#[test]
fn process_and_clock_helper_share_the_same_single_step() {
    let (mut driver, _observed, clock) = setup(Duration::ZERO);
    let (accepted, _response) = unbounded();
    put(&mut driver, Message::Advance(Advance(5, accepted)));
    assert!(driver.process().unwrap());
    assert_eq!(driver.queued(), 0);
    assert!(!driver.process().unwrap());
    let mut clock_reads = 0;
    let report = driver
        .process_with_clock(FrameBudget { duration: Duration::from_secs(3600), max_steps: 1000 }, || {
            clock_reads += 1;
            clock.get() + Duration::from_secs(clock_reads)
        })
        .unwrap();
    assert_eq!(report.stopped, StopReason::NoReadyWork);
    assert_eq!(report.steps, 0);
    assert!(clock_reads < 10, "no work must return without consuming the time allowance");
}
