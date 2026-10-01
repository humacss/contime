use std::sync::{Arc, OnceLock};
use std::time::Duration;

use contime_core::{checkpoints::*, ConTime, ConTimeConfig, Input, Placement, ProgressObservation, ProgressPolicy};
use crossbeam_channel::{unbounded, Receiver, Sender};

const WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Time(u64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, other: &Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}

struct Event {
    time: Time,
    id: u128,
    snapshot: u128,
}
impl contime_checkpoints::Event for Event {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl Input for Event {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot);
    }
}
fn event(time: u64) -> Event {
    Event { time: Time(time), id: time.into(), snapshot: (time % 2).into() }
}

#[derive(Clone, Default)]
struct State {
    time: Time,
    count: usize,
}
impl Snapshot for State {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl ApplyEvents<Event> for State {
    fn create(_: u128, _: &Event) -> Self {
        Self::default()
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Event>) {
        self.count += batch.events.count();
    }
}

struct Observe {
    reports: Sender<ProgressObservation<Time>>,
    lag: u64,
}
impl ProgressPolicy<Time> for Observe {
    fn cutoff(&mut self, advanced: &Time) -> Option<Time> {
        Some(Time(advanced.0.saturating_sub(self.lag)))
    }
    fn observed(&mut self, report: ProgressObservation<Time>) {
        let _ = self.reports.send(report);
    }
}
fn policy(lag: u64) -> (Observe, Receiver<ProgressObservation<Time>>) {
    let (reports, receiver) = unbounded();
    (Observe { reports, lag }, receiver)
}
fn settings() -> ConTimeConfig<Time> {
    ConTimeConfig {
        router_count: 2,
        worker_count: 2,
        placement: Placement::default(),
        history_retention: Time(1000),
        pruning_interval: Duration::from_millis(1),
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_millis(1),
            replays_per_receive: 1,
            deadline_compaction_minimum: 1024,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: CheckpointConfig { interval: 1 },
    }
}
fn reached(reports: &Receiver<ProgressObservation<Time>>, boundary: u64) -> ProgressObservation<Time> {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let report = reports.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())).unwrap();
        assert!(report.before <= report.cutoff);
        if report.before == Time(boundary) {
            return report;
        }
    }
}

#[test]
fn reports_beyond_pruning_without_removing_history_or_rejecting_late_input() {
    let (policy, reports) = policy(800);
    let core = ConTime::<Event, State, ()>::start_with_progress(settings(), (), policy).unwrap();
    core.apply([event(200)]).unwrap();
    core.advance_to(Time(1100)).unwrap();
    let first = reached(&reports, 300);
    core.wait_until_idle(WAIT).unwrap();
    assert_eq!(core.subscribe_pruned_horizon().unwrap().recv_timeout(WAIT).unwrap(), Time(100));
    assert_eq!(core.query_events_between(0, Time(100), Time(300)).unwrap().len(), 1);

    // This remains an ordinary admissible late event, even after report 300.
    core.apply([event(202)]).unwrap();
    let next = reached(&reports, 300);
    assert!(next.round > first.round);
    core.wait_until_idle(WAIT).unwrap();
    assert_eq!(core.query_at(Time(300), [0]).unwrap()[0].count, 2);
    assert!(core.errors().is_empty());
    assert_eq!(core.subscribe_pruned_horizon().unwrap().recv_timeout(WAIT).unwrap(), Time(100));
    core.shutdown();
}

#[test]
fn empty_intervals_are_reported_without_pruning_or_periodic_idle_work() {
    let (policy, reports) = policy(100);
    let core = ConTime::<Event, State, ()>::start_with_progress(settings(), (), policy).unwrap();
    core.advance_to(Time(400)).unwrap();
    reached(&reports, 300);
    core.wait_until_idle(WAIT).unwrap();
    reports.try_iter().for_each(drop);
    assert!(reports.recv_timeout(Duration::from_millis(20)).is_err());
    assert_eq!(core.subscribe_pruned_horizon().unwrap().recv_timeout(WAIT).unwrap(), Time(0));
    core.advance_to(Time(500)).unwrap();
    reached(&reports, 400);
    core.wait_until_idle(WAIT).unwrap();
    core.shutdown();
}

struct BeyondTarget(Sender<ProgressObservation<Time>>);
impl ProgressPolicy<Time> for BeyondTarget {
    fn cutoff(&mut self, _: &Time) -> Option<Time> {
        Some(Time(10_000))
    }
    fn observed(&mut self, report: ProgressObservation<Time>) {
        let _ = self.0.send(report);
    }
}

#[test]
fn policy_cannot_certify_time_beyond_the_processing_target() {
    let (sender, reports) = unbounded();
    let core = ConTime::<Event, State, ()>::start_with_progress(settings(), (), BeyondTarget(sender)).unwrap();
    core.apply([event(600)]).unwrap();
    core.advance_to(Time(400)).unwrap();
    let report = reached(&reports, 400);
    assert_eq!(report.cutoff, Time(400));
    core.wait_until_idle(WAIT).unwrap();
    core.advance_to(Time(700)).unwrap();
    reached(&reports, 700);
    assert_eq!(core.query_at(Time(700), [0]).unwrap()[0].count, 1);
    core.wait_until_idle(WAIT).unwrap();
    core.shutdown();
}

#[derive(Clone)]
struct Block {
    entered: Sender<()>,
    release: Receiver<()>,
}
impl ApplyWrapper<State, Event> for Block {
    fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, Time, Event>, inner: &mut ApplyInner<'_, State>) {
        inner.apply_event_batch(batch);
        self.entered.send(()).unwrap();
        self.release.recv_timeout(WAIT).unwrap();
    }
}

#[test]
fn blocked_processing_inside_buffer_prevents_a_complete_report() {
    let (policy, reports) = policy(100);
    let (entered, arrivals) = unbounded();
    let (release, blocked) = unbounded();
    let core = ConTime::start_with_progress(settings(), Block { entered, release: blocked }, policy).unwrap();
    core.apply([event(200)]).unwrap();
    core.advance_to(Time(400)).unwrap();
    arrivals.recv_timeout(WAIT).unwrap();
    let report = reports.recv_timeout(Duration::from_millis(20));
    // Release before assertions so a failed test does not strand a worker.
    release.send(()).unwrap();
    if let Ok(report) = report {
        assert!(report.before <= Time(200));
    }
    reached(&reports, 300);
    core.wait_until_idle(WAIT).unwrap();
    assert_eq!(core.query_at(Time(300), [0]).unwrap()[0].count, 1);
    core.shutdown();
}

type Emit = Arc<OnceLock<Box<dyn Fn(u64) + Send + Sync>>>;
#[derive(Clone)]
struct Feedback(Emit);
impl ApplyWrapper<State, Event> for Feedback {
    fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, Time, Event>, inner: &mut ApplyInner<'_, State>) {
        let time = batch.time.0;
        inner.apply_event_batch(batch);
        (self.0.get().unwrap())(time);
    }
}

#[test]
fn cross_worker_feedback_settles_with_no_further_advancement() {
    let (policy, reports) = policy(100);
    let emit: Emit = Arc::new(OnceLock::new());
    let core = Arc::new(ConTime::start_with_progress(settings(), Feedback(emit.clone()), policy).unwrap());
    let weak = Arc::downgrade(&core);
    assert!(emit
        .set(Box::new(move |time| {
            if time < 210 {
                weak.upgrade().unwrap().apply_internal(Time(time), [event(time + 1)]).unwrap();
            }
        }))
        .is_ok());
    core.apply([event(200)]).unwrap();
    core.advance_to(Time(400)).unwrap();
    reached(&reports, 300);
    // The report itself must cover feedback, not merely the later idle wait.
    let count: usize = core.query_at(Time(300), [0, 1]).unwrap().iter().map(|s| s.count).sum();
    assert_eq!(count, 11);
    core.wait_until_idle(WAIT).unwrap();
    assert!(core.errors().is_empty());
    Arc::try_unwrap(core).ok().unwrap().shutdown();
}

#[test]
fn internal_feedback_before_a_previous_observation_remains_admissible() {
    let (policy, reports) = policy(100);
    let emit: Emit = Arc::new(OnceLock::new());
    let core = Arc::new(ConTime::start_with_progress(settings(), Feedback(emit.clone()), policy).unwrap());
    let weak = Arc::downgrade(&core);
    assert!(emit
        .set(Box::new(move |time| {
            if time < 210 {
                weak.upgrade().unwrap().apply_internal(Time(time), [event(time + 1)]).unwrap();
            }
        }))
        .is_ok());
    core.advance_to(Time(400)).unwrap();
    let previous = reached(&reports, 300);
    core.wait_until_idle(WAIT).unwrap();
    reports.try_iter().for_each(drop);

    // New retained input can generate internal work before an old observation.
    // The observation must not become a new admission-rejection frontier.
    core.apply([event(200)]).unwrap();
    let next = reached(&reports, 300);
    assert!(next.round > previous.round);
    let count: usize = core.query_at(Time(300), [0, 1]).unwrap().iter().map(|s| s.count).sum();
    assert_eq!(count, 11);
    core.wait_until_idle(WAIT).unwrap();
    assert!(core.errors().is_empty());
    Arc::try_unwrap(core).ok().unwrap().shutdown();
}
