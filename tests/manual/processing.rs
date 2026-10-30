use contime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot, Timestamp};
use contime::manual::{FrameBudget, ManualConfig, RawManualConTime};
use std::{cell::RefCell, num::NonZeroUsize, rc::Rc, time::Duration};
#[derive(Clone, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);
impl Timestamp for Time {
    fn minimum() -> Self {
        Self(0)
    }
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime::core::AdvanceTime for Time {
    fn is_valid_retention(&self) -> bool {
        true
    }
    fn saturating_sub(&self, rhs: &Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}
struct Change {
    id: u128,
    time: Time,
    targets: Vec<u128>,
    value: Rc<u64>,
}
impl Event for Change {
    type Time = Time;
    fn time(&self) -> Time {
        self.time.clone()
    }
}
impl contime::Input for Change {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        for id in &self.targets {
            emit(*id)
        }
    }
}
#[derive(Clone, Default)]
struct Counter {
    time: Time,
    id: u128,
    values: Vec<u64>,
}
impl Snapshot for Counter {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time
    }
}
impl ApplyEvents<Change> for Counter {
    fn create(id: u128, _: &Change) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Change>) {
        self.values.extend(batch.events.map(|e| *e.value));
    }
}
fn engine(capacity: usize) -> RawManualConTime<Change, Counter> {
    RawManualConTime::new(
        ManualConfig { queue_capacity: NonZeroUsize::new(capacity).unwrap(), checkpoints: CheckpointConfig { interval: 1 } },
        (),
    )
}
fn event(id: u128, time: u64, value: u64, targets: &[u128]) -> Change {
    Change { id, time: Time(time), targets: targets.into(), value: Rc::new(value) }
}
fn process(engine: &mut RawManualConTime<Change, Counter>) {
    engine.process_with_clock(FrameBudget { duration: Duration::from_secs(1), max_steps: 1000 }, || Duration::ZERO).unwrap();
}
#[test]
fn manual_api_routes_replays_and_queries_without_threads_or_send_bounds() {
    let mut engine = engine(8);
    engine.apply([event(1, 5, 50, &[1, 2]), event(2, 10, 100, &[1])]).unwrap();
    engine.advance_to(Time(11)).unwrap();
    process(&mut engine);
    engine.apply([event(3, 7, 70, &[1]), event(1, 5, 50, &[1, 2])]).unwrap();
    let observed = Rc::new(RefCell::new(Vec::new()));
    let output = observed.clone();
    engine.query_at(Time(11), [1, 2], move |states| *output.borrow_mut() = states).unwrap();
    assert!(observed.borrow().is_empty());
    process(&mut engine);
    let states = observed.borrow();
    assert_eq!(states.iter().find(|s| s.id == 1).unwrap().values, [50, 70, 100]);
    assert_eq!(states.iter().find(|s| s.id == 2).unwrap().values, [50]);
    assert!(engine.take_rejections().is_empty());
}
#[test]
fn queue_rejection_and_dropping_queued_queries_never_invoke_callbacks() {
    let mut engine = engine(1);
    let called = Rc::new(RefCell::new(0));
    let count = called.clone();
    engine.query_at(Time(1), [1], move |_| *count.borrow_mut() += 1).unwrap();
    let count = called.clone();
    assert!(engine.query_at(Time(1), [1], move |_| *count.borrow_mut() += 1).is_err());
    drop(engine);
    assert_eq!(*called.borrow(), 0);
}
#[test]
fn empty_queries_complete_and_pruning_reports_rejected_event_identity() {
    let mut engine = engine(16);
    let empty = Rc::new(RefCell::new(None));
    let out = empty.clone();
    engine.query_at(Time(0), [99], move |states| *out.borrow_mut() = Some(states.len())).unwrap();
    process(&mut engine);
    assert_eq!(*empty.borrow(), Some(0));
    engine.apply([event(1, 5, 50, &[1]), event(2, 10, 100, &[1])]).unwrap();
    engine.advance_to(Time(11)).unwrap();
    engine.prune_before(Time(8)).unwrap();
    engine.apply([event(44, 7, 99, &[1]), event(45, 2, 98, &[1])]).unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let out = events.clone();
    engine.query_events_between(1, Time(0), Time(20), move |values| *out.borrow_mut() = values).unwrap();
    process(&mut engine);
    let rejected = engine.take_rejections();
    assert_eq!(rejected.iter().map(|item| item.event_id).collect::<Vec<_>>(), [44, 45]);
    assert_eq!(events.borrow().iter().map(|e| contime::Input::event_id(&**e)).collect::<Vec<_>>(), [2]);
}

#[test]
fn missing_and_empty_event_queries_complete_with_empty_results() {
    let mut engine = engine(8);
    engine.apply([event(1, 5, 50, &[1])]).unwrap();
    let observed = Rc::new(RefCell::new(Vec::new()));
    for (id, from, to) in [(99, 0, 20), (1, 5, 5), (1, 10, 1)] {
        let output = observed.clone();
        engine.query_events_between(id, Time(from), Time(to), move |values| output.borrow_mut().push(values.len())).unwrap();
    }
    process(&mut engine);
    assert_eq!(*observed.borrow(), [0, 0, 0]);
}

#[test]
fn panicking_callbacks_propagate_and_dropping_the_instance_cancels_remaining_queries() {
    let mut engine = engine(8);
    let calls = Rc::new(RefCell::new(0));
    let first = calls.clone();
    engine
        .query_at(Time(0), [1], move |_| {
            *first.borrow_mut() += 1;
            panic!("display callback failed");
        })
        .unwrap();
    let later = calls.clone();
    engine.query_at(Time(0), [1], move |_| *later.borrow_mut() += 100).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process(&mut engine)));
    assert!(result.is_err());
    drop(engine);
    assert_eq!(*calls.borrow(), 1);
}

#[test]
fn process_handles_one_queued_operation_then_returns_to_the_host() {
    let mut contime = engine(8);
    let callbacks = Rc::new(RefCell::new(Vec::new()));
    for id in [1, 2] {
        let output = callbacks.clone();
        contime.query_at(Time(0), [id], move |_| output.borrow_mut().push(id)).unwrap();
    }
    assert!(contime.process().unwrap());
    assert_eq!(*callbacks.borrow(), [1]);
    assert_eq!(contime.queued(), 1);
    assert!(contime.process().unwrap());
    assert_eq!(*callbacks.borrow(), [1, 2]);
    assert!(!contime.process().unwrap());
}

#[test]
fn worker_status_observes_manual_work_without_driving_it() {
    let mut engine = engine(8);
    engine.apply([event(1, 5, 50, &[1, 2])]).unwrap();
    engine.advance_to(Time(10)).unwrap();
    let queued = engine.queued();
    for _ in 0..2 {
        let status = engine.worker_status();
        assert_eq!(status.worker_id, 0);
        assert_eq!(status.history_stores, 0);
        assert_eq!(status.processing_target.0, 0);
        assert_eq!(engine.queued(), queued);
        #[cfg(feature = "monitor")]
        assert_eq!(status.counters.unwrap().inserted_applications, 0);
        #[cfg(not(feature = "monitor"))]
        assert!(status.counters.is_none());
    }
    assert!(engine.process().unwrap()); // insertion only
    let status = engine.worker_status();
    assert_eq!(status.history_stores, 2);
    #[cfg(feature = "monitor")]
    {
        let counters = status.counters.unwrap();
        assert_eq!(counters.inserted_applications, 2);
        assert_eq!(counters.replay_applications, Some(0));
    }
    process(&mut engine);
    let status = engine.worker_status();
    assert_eq!(status.processing_target.0, 10);
    assert_eq!(status.pending_histories, 0);
    assert_eq!(status.due_histories, 0);
    #[cfg(feature = "monitor")]
    assert_eq!(status.counters.unwrap().replay_applications, Some(2));
    engine.prune_before(Time(6)).unwrap();
    process(&mut engine);
    assert_eq!(engine.worker_status().admission_horizon.0, 6);
}
