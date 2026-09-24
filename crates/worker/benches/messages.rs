use std::cell::Cell;
use std::collections::VecDeque;
use std::hint::black_box;
use std::rc::Rc;
use std::time::Duration;

use contime_worker::{
    work_messages, AdvanceInput, ApplyBatch, Completion, EventInsert, EventQueryInput, EventQueryResponse, RoutedInput,
    SnapshotListenInput, SnapshotListener, SnapshotQueryInput, SnapshotQueryResponse, SnapshotStore, WorkInput, WorkInputKind,
    WorkerConfig,
};
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use crossbeam_channel::{unbounded, Receiver, Sender};

type Time = u64;

#[derive(Clone)]
struct TestEvent {
    time: Time,
    value: u64,
}

#[derive(Default)]
struct Observed {
    inserted: Cell<u64>,
    processed: Cell<u64>,
    sum: Cell<u64>,
    batches: Cell<u64>,
    queries: Cell<u64>,
    event_queries: Cell<u64>,
    query_results: Cell<u64>,
    event_results: Cell<u64>,
    query_sum: Cell<u64>,
    event_sum: Cell<u64>,
    forwarded: Cell<u64>,
    pruned: Cell<u64>,
}

struct TestStore {
    pending: VecDeque<TestEvent>,
    sum: u64,
    observed: Rc<Observed>,
}

// Intentionally not a real history: constant-time queueing/front lookup and a
// running sum expose worker costs without checkpoint selection, dedup or replay.
impl SnapshotStore<TestEvent> for TestStore {
    type Config = Rc<Observed>;
    type Context = Context;
    type Time = Time;
    type Snapshot = u64;
    type Rejection = ();

    fn create(_: u128, observed: &Rc<Observed>, _: &Time) -> Self {
        Self { pending: VecDeque::new(), sum: 0, observed: observed.clone() }
    }
    fn insert(&mut self, event: TestEvent, _: &Time) -> EventInsert<()> {
        self.observed.inserted.set(self.observed.inserted.get() + 1);
        self.pending.push_back(event);
        EventInsert { changed: true, rejections: vec![] }
    }
    fn event_time(event: &TestEvent) -> Time {
        event.time
    }
    fn process_until(&mut self, time: &Time, context: &mut Context) {
        while self.pending.front().is_some_and(|event| event.time <= *time) {
            let event = self.pending.pop_front().unwrap();
            self.sum += event.value;
            self.observed.sum.set(self.observed.sum.get() + event.value);
            self.observed.processed.set(self.observed.processed.get() + 1);
        }
        self.observed.batches.set(self.observed.batches.get() + 1);
        if self.observed.processed.get() == context.next_stage {
            for message in context.stages.pop_front().unwrap() {
                context.sender.as_ref().unwrap().send(message).unwrap();
            }
            context.next_stage += context.events_per_round;
            if context.stages.is_empty() {
                context.sender.take();
            }
        }
    }
    fn query(&mut self, _: Time, _: &mut Context) -> Option<Box<u64>> {
        Some(Box::new(self.sum))
    }
    fn query_events(&self, from: &Time, _: &Time) -> Vec<TestEvent> {
        vec![TestEvent { time: *from, value: self.sum }]
    }
    fn forward(&mut self, _: &Time, _: &mut Context) {
        self.observed.forwarded.set(self.observed.forwarded.get() + 1);
    }
    fn prune(&mut self) {
        self.observed.pruned.set(self.observed.pruned.get() + 1);
    }
}

struct Context {
    sender: Option<Sender<Message>>,
    stages: VecDeque<Vec<Message>>,
    next_stage: u64,
    events_per_round: u64,
}

struct Done;
impl Completion<()> for Done {
    fn reject(self, _: Vec<()>) {
        panic!("benchmark inputs must be accepted");
    }
}
struct Query(Rc<Observed>, u128, Time);
impl SnapshotQueryInput for Query {
    type Time = Time;
    type Response = Self;
    fn into_parts(self) -> (Time, Vec<u128>, Self) {
        (self.2, vec![self.1], self)
    }
}
impl SnapshotQueryResponse<u64> for Query {
    fn send(self, snapshots: Vec<Box<u64>>) {
        self.0.query_results.set(self.0.query_results.get() + snapshots.len() as u64);
        self.0.query_sum.set(self.0.query_sum.get() + snapshots.iter().map(|snapshot| **snapshot).sum::<u64>());
        black_box(snapshots);
        self.0.queries.set(self.0.queries.get() + 1);
    }
}
struct EventQuery(Rc<Observed>, u128, Time);
impl EventQueryInput for EventQuery {
    type Time = Time;
    type Response = Self;
    fn into_parts(self) -> (u128, Time, Time, Self) {
        (self.1, 0, self.2, self)
    }
}
impl EventQueryResponse<TestEvent> for EventQuery {
    fn send(self, events: Vec<TestEvent>) {
        self.0.event_results.set(self.0.event_results.get() + events.len() as u64);
        self.0.event_sum.set(self.0.event_sum.get() + events.iter().map(|event| event.value).sum::<u64>());
        black_box(events);
        self.0.event_queries.set(self.0.event_queries.get() + 1);
    }
}
#[derive(Clone)]
struct Listener;
impl SnapshotListener<Time> for Listener {
    fn registered(&self, _: Time, _: Vec<u128>) -> bool {
        true
    }
    fn replayed(&self, _: Time, _: Vec<u128>) -> bool {
        true
    }
}
struct Listen;
impl SnapshotListenInput for Listen {
    type Time = Time;
    type Listener = Listener;
    fn into_parts(self) -> (Time, Vec<u128>, Listener) {
        (0, vec![], Listener)
    }
}
struct Advance(Time);
impl AdvanceInput for Advance {
    type Time = Time;
    type Completion = ();
    fn into_parts(self) -> (Time, ()) {
        (self.0, ())
    }
}
type Kind = WorkInputKind<ApplyBatch<TestEvent, Done>, Query, EventQuery, Listen, Advance>;
struct Message(Kind);
impl WorkInput for Message {
    type Apply = ApplyBatch<TestEvent, Done>;
    type SnapshotQuery = Query;
    type EventQuery = EventQuery;
    type SnapshotListen = Listen;
    type Advance = Advance;
    fn into_kind(self) -> Kind {
        self.0
    }
}

#[derive(Clone, Copy)]
struct Scenario {
    events: u64,
    snapshots: u64,
    rounds: u64,
    distinct_times: bool,
    process: bool,
    queries: u64,
    prune: bool,
}
struct Fixture {
    receiver: Receiver<Message>,
    observed: Rc<Observed>,
    context: Context,
}

fn inputs(scenario: Scenario, round: u64) -> Message {
    let inputs = (0..scenario.events)
        .map(|index| RoutedInput {
            snapshot_id: (index % scenario.snapshots) as u128,
            input: TestEvent { time: round * scenario.events + if scenario.distinct_times { index + 1 } else { 1 }, value: 1 },
        })
        .collect();
    Message(Kind::Apply(ApplyBatch { inputs, completion: Done }))
}
fn fixture(scenario: Scenario) -> Fixture {
    let observed = Rc::new(Observed::default());
    let (sender, receiver) = unbounded::<Message>();
    sender.send(inputs(scenario, 0)).unwrap();
    let mut stages = VecDeque::new();
    if scenario.process {
        sender.send(Message(Kind::Advance(Advance(scenario.events)))).unwrap();
        for round in 0..scenario.rounds {
            let target = (round + 1) * scenario.events;
            let mut stage = Vec::new();
            for query in 0..scenario.queries {
                let id = (query % scenario.snapshots) as u128;
                stage.push(Message(Kind::SnapshotQuery(Query(observed.clone(), id, target))));
                stage.push(Message(Kind::EventQuery(EventQuery(observed.clone(), id, target))));
            }
            if scenario.prune {
                stage.push(Message(Kind::Prune(Advance(target + 1))));
            }
            if round + 1 < scenario.rounds {
                stage.push(inputs(scenario, round + 1));
                stage.push(Message(Kind::Advance(Advance(target + scenario.events))));
            }
            stages.push_back(stage);
        }
    }
    Fixture {
        receiver,
        observed,
        context: Context {
            sender: scenario.process.then_some(sender),
            stages,
            next_stage: scenario.events,
            events_per_round: scenario.events,
        },
    }
}
fn run(fixture: Fixture) -> Rc<Observed> {
    let config = WorkerConfig {
        maximum_dirty_age: Duration::ZERO,
        replays_per_receive: 0,
        deadline_compaction_minimum: 0,
        deadline_compaction_multiplier: 0,
    };
    work_messages::<_, TestStore>(fixture.receiver, config, fixture.observed.clone(), 0, fixture.context, crossbeam_channel::never(), None);
    black_box(fixture.observed)
}
fn verify(scenario: Scenario) {
    let actual = run(fixture(scenario));
    let inserted = scenario.events * scenario.rounds;
    let processed = if scenario.process { inserted } else { 0 };
    let batches = if !scenario.process { 0 } else { scenario.snapshots * scenario.rounds };
    assert_eq!(actual.inserted.get(), inserted);
    assert_eq!(actual.processed.get(), processed);
    assert_eq!(actual.sum.get(), processed);
    assert_eq!(actual.batches.get(), batches);
    assert_eq!(actual.queries.get(), scenario.queries * scenario.rounds);
    assert_eq!(actual.event_queries.get(), scenario.queries * scenario.rounds);
    assert_eq!(actual.query_results.get(), scenario.queries * scenario.rounds);
    assert_eq!(actual.event_results.get(), scenario.queries * scenario.rounds);
    let expected_query_sum = scenario.queries * (scenario.events / scenario.snapshots) * scenario.rounds * (scenario.rounds + 1) / 2;
    assert_eq!(actual.query_sum.get(), expected_query_sum);
    assert_eq!(actual.event_sum.get(), expected_query_sum);
    assert_eq!(actual.forwarded.get(), if scenario.prune { scenario.snapshots * scenario.rounds } else { 0 });
    assert_eq!(actual.pruned.get(), actual.forwarded.get());
}
fn benchmark_messages(criterion: &mut Criterion) {
    let base = Scenario { events: 1_000, snapshots: 1, rounds: 1, distinct_times: false, process: true, queries: 0, prune: false };
    let scenarios = [
        ("insert_future/1000", Scenario { process: false, ..base }),
        ("same_timestamp/1000/1_snapshot", base),
        ("distinct_timestamps/1000/1_snapshot", Scenario { distinct_times: true, ..base }),
        ("distinct_timestamps/10000/1_snapshot", Scenario { events: 10_000, distinct_times: true, ..base }),
        ("same_timestamp/1000/1000_snapshots", Scenario { snapshots: 1_000, ..base }),
        ("queries/1000_pairs", Scenario { queries: 1_000, ..base }),
        ("prune/1000_snapshots", Scenario { snapshots: 1_000, prune: true, ..base }),
        ("mixed/100_rounds", Scenario { events: 100, snapshots: 10, rounds: 100, queries: 10, prune: true, ..base }),
    ];
    let mut group = criterion.benchmark_group("worker/messages");
    group.sample_size(20).warm_up_time(Duration::from_millis(200)).measurement_time(Duration::from_secs(1));
    for (name, scenario) in scenarios {
        verify(scenario);
        group.throughput(Throughput::Elements(scenario.events * scenario.rounds));
        group.bench_function(name, |bencher| {
            bencher.iter_batched(
                || fixture(scenario),
                |fixture| {
                    black_box(run(black_box(fixture)));
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, benchmark_messages);
criterion_main!(benches);
