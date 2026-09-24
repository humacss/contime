use contime_snapshots::{Apply, Checkpoint, Event, EventStore, NoCheckpoint, Snapshot, SnapshotStore, Timestamp};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Time(u64);
#[derive(Clone, Debug, PartialEq, Eq)]
struct TestSnapshot {
    time: Time,
    sum: u64,
}
struct TestEvent(Time, u64);
struct TestEventStore(Vec<TestEvent>, Rc<Cell<usize>>);

impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl Snapshot for TestSnapshot {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl Event for TestEvent {
    type Time = Time;
    fn time(&self) -> Time {
        self.0
    }
}
impl EventStore for TestEventStore {
    type Time = Time;
    type Event = TestEvent;
    type Iter<'a> = std::slice::Iter<'a, TestEvent>;
    fn iter_after(&self, boundary: Option<&Time>) -> Self::Iter<'_> {
        let start = boundary.map_or(0, |time| self.0.partition_point(|event| event.0 <= *time));
        self.0[start..].iter()
    }
    fn prune_before(&mut self, horizon: &Time) {
        let end = self.0.partition_point(|event| event.0 < *horizon);
        self.0.drain(..end);
        self.1.set(self.1.get() + end);
    }
}
impl Apply<Checkpoint<TestSnapshot>, Cell<usize>> for TestEvent {
    fn apply<'a>(checkpoint: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, calls: &Cell<usize>) {
        calls.set(calls.get() + 1);
        checkpoint.snapshot.sum += events.map(|event| event.1).sum::<u64>();
    }
}

#[rstest::rstest]
#[case::without_prior_replay(false)]
#[case::with_prior_replay(true)]
fn happy(#[case] replay_first: bool) {
    let expected_sum = 7;
    let expected_error = NoCheckpoint;
    let expected_hook_time = Time(19);

    let events = TestEventStore(vec![TestEvent(Time(10), 1), TestEvent(Time(20), 2), TestEvent(Time(30), 4)], Rc::default());
    let mut snapshots = SnapshotStore::new(events, TestSnapshot { time: Time(0), sum: 0 }, 1);
    let context = Cell::new(0);
    let mut hooks = Vec::new();

    if replay_first {
        snapshots.process_until(Time(30), &context).unwrap();
    }
    snapshots.forward(Time(20), &context, |_, time, _| hooks.push(*time)).unwrap();
    let expired = snapshots.query(Time(19), &context);
    snapshots.process_until(Time(30), &context).unwrap();
    let actual = snapshots.query(Time(30), &context).unwrap();

    assert_eq!(actual.sum, expected_sum);
    assert_eq!(expired, Err(expected_error));
    assert_eq!(hooks.last(), Some(&expected_hook_time));
}

#[test]
fn forwarding_across_a_gap_runs_the_final_hook() {
    let expected_sum = 3;
    let expected_time = Time(19);

    let events = TestEventStore(vec![TestEvent(Time(10), 3)], Rc::default());
    let mut snapshots = SnapshotStore::new(events, TestSnapshot { time: Time(0), sum: 0 }, 100);
    let context = Cell::new(0);
    let mut hook_times = Vec::new();

    snapshots.forward(Time(20), &context, |_, time, _| hook_times.push(*time)).unwrap();
    snapshots.prune();
    let actual = snapshots.query(Time(20), &context).unwrap();

    assert_eq!(actual.sum, expected_sum);
    assert_eq!(actual.time, expected_time);
    assert_eq!(hook_times.last(), Some(&expected_time));
}

#[test]
fn multiple_forwards_can_be_followed_by_one_prune() {
    let expected = TestSnapshot { time: Time(30), sum: 7 };
    let expected_error = NoCheckpoint;

    let events = TestEventStore(vec![TestEvent(Time(10), 1), TestEvent(Time(20), 2), TestEvent(Time(30), 4)], Rc::default());
    let mut snapshots = SnapshotStore::new(events, TestSnapshot { time: Time(0), sum: 0 }, 1);
    let context = Cell::new(0);

    snapshots.prune();
    snapshots.forward(Time(15), &context, |_, _, _| {}).unwrap();
    snapshots.forward(Time(30), &context, |_, _, _| {}).unwrap();
    let calls_before_prune = context.get();
    snapshots.prune();
    snapshots.prune();
    let calls_after_prune = context.get();
    let expired = snapshots.query(Time(29), &context);
    snapshots.process_until(Time(30), &context).unwrap();
    let actual = snapshots.query(Time(30), &context).unwrap();

    assert_eq!(*actual, expected);
    assert_eq!(expired, Err(expected_error));
    assert_eq!(calls_after_prune, calls_before_prune);
}

#[rstest::rstest]
#[case::prune_each_time(true)]
#[case::deferred_prune(false)]
fn pruning_removes_only_expired_events(#[case] prune_each_time: bool) {
    let expected_removed_before_prune = if prune_each_time { 1 } else { 0 };
    let expected_removed = 2;
    let expected_total_removed = 4;
    let expected_sum = 10;

    let removed = Rc::new(Cell::new(0));
    let events = TestEventStore(
        vec![TestEvent(Time(10), 1), TestEvent(Time(20), 2), TestEvent(Time(30), 3), TestEvent(Time(30), 4)],
        removed.clone(),
    );
    let mut snapshots = SnapshotStore::new(events, TestSnapshot { time: Time(0), sum: 0 }, 1);
    let context = Cell::new(0);

    snapshots.forward(Time(20), &context, |_, _, _| {}).unwrap();
    if prune_each_time {
        snapshots.prune();
    }
    snapshots.forward(Time(30), &context, |_, _, _| {}).unwrap();
    let removed_before_prune = removed.get();
    let applications_before_prune = context.get();
    snapshots.prune();
    snapshots.prune();
    let applications_after_prune = context.get();
    let removed_after_prune = removed.get();
    let at_horizon = snapshots.query(Time(30), &context).unwrap();
    snapshots.forward(Time(31), &context, |_, _, _| {}).unwrap();
    snapshots.prune();
    let actual = snapshots.query(Time(31), &context).unwrap();

    assert_eq!(removed_before_prune, expected_removed_before_prune);
    assert_eq!(removed_after_prune, expected_removed);
    assert_eq!(applications_after_prune, applications_before_prune);
    assert_eq!(at_horizon.sum, expected_sum);
    assert_eq!(actual.sum, expected_sum);
    assert_eq!(removed.get(), expected_total_removed);
}

#[test]
fn horizon_without_predecessor_leaves_history_unchanged() {
    let expected_error = contime_snapshots::ForwardError::NoPreviousTimestamp;
    let expected_sum = 3;
    let expected_hook_count = 0;

    let events = TestEventStore(vec![TestEvent(Time(0), expected_sum)], Rc::default());
    let mut snapshots = SnapshotStore::new(events, TestSnapshot { time: Time(0), sum: 0 }, 1);
    let context = Cell::new(0);
    let mut hook_count = 0;

    let error = snapshots.forward(Time(0), &context, |_, _, _| hook_count += 1);
    let actual = snapshots.query(Time(0), &context).unwrap();

    assert_eq!(error, Err(expected_error));
    assert_eq!(actual.sum, expected_sum);
    assert_eq!(hook_count, expected_hook_count);
}
