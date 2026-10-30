use contime_api::ApplyOutput;
use contime_router::{AdvanceWorkerOutput, RouteOutput, WorkerOutput};
use contime_runtime::checkpoints::{ApplyBatch, ApplyEvents, Snapshot};
use contime_runtime::{
    CompletionHandle, Input, RejectionReason, Route, RouterBatch, RouterMessage, RouterProcess, SharedEvent, WorkerBatch, WorkerMessage,
    WorkerProcess,
};
use contime_runtime::{Router, Worker};
use crossbeam_channel::{unbounded, Receiver, Sender, TryRecvError};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(i64);
impl contime_runtime::checkpoints::Timestamp for Time {
    // This fixture models a nonnegative logical clock.
    fn minimum() -> Self {
        Self(0)
    }
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).filter(|time| *time >= 0).map(Self)
    }
}
impl contime_worker::AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool {
        self.0 >= 0
    }
    fn saturating_sub(&self, t: &Self) -> Self {
        Self(self.0.saturating_sub(t.0).max(0))
    }
}
struct Gate {
    entered: Sender<()>,
    release: Receiver<()>,
    used: AtomicBool,
}
struct Event {
    id: u128,
    time: Time,
    histories: Vec<u128>,
    gate: Option<Arc<Gate>>,
}
impl contime_runtime::checkpoints::Event for Event {
    type Time = Time;
    fn time(&self) -> Time {
        if let Some(gate) = &self.gate {
            if !gate.used.swap(true, Ordering::SeqCst) {
                gate.entered.send(()).unwrap();
                gate.release.recv().unwrap();
            }
        }
        self.time
    }
}
impl Input for Event {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for &id in &self.histories {
            emit(id);
        }
    }
}
#[derive(Clone, Default)]
struct State {
    time: Time,
}
impl Snapshot for State {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, t: Time) {
        self.time = t;
    }
}
impl ApplyEvents<Event> for State {
    fn create(_: u128, _: &Event) -> Self {
        Self::default()
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Event>) {
        for _ in batch.events {}
    }
}
fn event(id: u128, time: i64, histories: Vec<u128>) -> SharedEvent<Event> {
    Event { id, time: Time(time), histories, gate: None }.into()
}
fn worker() -> WorkerProcess<Event, State, ()> {
    WorkerProcess::new(contime_runtime::checkpoints::CheckpointConfig { interval: 100 }, ())
}
fn segment(input: SharedEvent<Event>, response: Sender<contime_api::RejectionMessage<RejectionReason>>) -> WorkerBatch<Event> {
    let routes = vec![Route::create(0, input)];
    WorkerBatch::create(routes, CompletionHandle::new(response))
}
#[test]
fn independent_response_closes_before_later_segment_can_finish() {
    let (entered, waiting) = unbounded();
    let (release, blocked) = unbounded();
    let gate = Arc::new(Gate { entered, release: blocked, used: AtomicBool::new(false) });
    let (a, completed_a) = unbounded();
    let (b, completed_b) = unbounded();
    let first = segment(event(1, 1, vec![0]), a);
    let second = segment(Event { id: 2, time: Time(2), histories: vec![0], gate: Some(gate) }.into(), b);
    let (input, incoming) = unbounded();
    input.send(WorkerMessage::ApplyPacket(vec![first, second])).ok().unwrap();
    drop(input);
    let thread = std::thread::spawn(move || worker().run(incoming).unwrap());
    waiting.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let a = completed_a.try_recv();
    let b = completed_b.try_recv();
    release.send(()).unwrap();
    thread.join().unwrap();
    assert_eq!(a, Err(TryRecvError::Disconnected));
    assert_eq!(b, Err(TryRecvError::Empty));
    assert_eq!(completed_b.try_recv(), Err(TryRecvError::Disconnected));
}
#[test]
fn identical_event_ids_keep_distinct_rejection_destinations() {
    let (a, completed_a) = unbounded();
    let (b, completed_b) = unbounded();
    let (input, incoming) = unbounded();
    let prune = <WorkerMessage<Event, State> as AdvanceWorkerOutput<_, _>>::advance(Time(10), unbounded().0);
    let WorkerMessage::Advance(prune) = prune else { unreachable!() };
    input.send(WorkerMessage::Prune(prune)).ok().unwrap();
    input.send(WorkerMessage::ApplyPacket(vec![segment(event(7, 9, vec![0]), a), segment(event(7, 10, vec![0]), b)])).ok().unwrap();
    drop(input);
    worker().run(incoming).unwrap();
    let rejected = completed_a.try_iter().collect::<Vec<_>>();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].reason, RejectionReason::BeforeHistoryHorizon);
    assert_eq!(completed_b.try_recv(), Err(TryRecvError::Disconnected));
}
#[test]
fn packet_routing_preserves_fanout_and_zero_route_completion() {
    let (a, completed_a) = unbounded();
    let (b, completed_b) = unbounded();
    let (input, incoming) = unbounded();
    input
        .send(RouterMessage::<Event, State>::ApplyPacket(vec![
            RouterBatch::create(vec![event(1, 1, vec![0, 1])], a),
            RouterBatch::create(vec![event(2, 1, vec![])], b),
        ]))
        .ok()
        .unwrap();
    drop(input);
    let (mut senders, mut receivers) = (Vec::new(), Vec::new());
    for _ in 0..2 {
        let (s, r) = unbounded();
        senders.push(s);
        receivers.push(r);
    }
    RouterProcess::new(Default::default()).run(incoming, senders).unwrap();
    assert_eq!(completed_b.try_recv(), Err(TryRecvError::Disconnected));
    worker().run(receivers.remove(0)).unwrap();
    assert_eq!(completed_a.try_recv(), Err(TryRecvError::Empty));
    worker().run(receivers.remove(0)).unwrap();
    assert_eq!(completed_a.try_recv(), Err(TryRecvError::Disconnected));
}
