use contime_snapshots::{Apply, Checkpoint, Event, EventStore, Insert, InsertEventStore, Snapshot, SnapshotStore, Timestamp};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Time(u64);
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TestSnapshot {
    time: Time,
    sum: u64,
}
struct TestEvent {
    id: u64,
    time: Time,
    value: u64,
}
#[derive(Default)]
struct TestEventStore(Vec<TestEvent>);
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
        self.time
    }
}
impl EventStore for TestEventStore {
    type Time = Time;
    type Event = TestEvent;
    type Iter<'a> = std::slice::Iter<'a, TestEvent>;
    fn iter_after(&self, boundary: Option<&Time>) -> Self::Iter<'_> {
        let start = boundary.map_or(0, |time| self.0.partition_point(|event| event.time <= *time));
        self.0[start..].iter()
    }
    fn prune_before(&mut self, horizon: &Time) {
        self.0.retain(|event| event.time >= *horizon);
    }
}
impl InsertEventStore for TestEventStore {
    fn insert(&mut self, event: TestEvent) -> Insert {
        if self.0.iter().any(|stored| stored.id == event.id) {
            return Insert::Duplicate;
        }
        let at = self.0.partition_point(|stored| (stored.time, stored.id) < (event.time, event.id));
        self.0.insert(at, event);
        Insert::Inserted
    }
}
impl Apply<Checkpoint<TestSnapshot>, Cell<usize>> for TestEvent {
    fn apply<'a>(
        checkpoint: &mut Checkpoint<TestSnapshot>,
        events: impl Iterator<Item = &'a Self>,
        _: contime_snapshots::BatchLookahead<'_, Time>,
        calls: &Cell<usize>,
    ) {
        calls.set(calls.get() + 1);
        checkpoint.snapshot.sum += events.map(|event| event.value).sum::<u64>();
    }
}

#[rstest::rstest]
#[case::late(10)]
#[case::same_timestamp(20)]
#[case::future(30)]
#[case::minimum_timestamp(0)]
fn happy(#[case] inserted_time: u64) {
    let expected_sum = 7;
    let expected_insert = Insert::Inserted;

    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), 1);
    let calls = Cell::new(0);
    store.insert(TestEvent { id: 1, time: Time(0), value: 1 });
    store.insert(TestEvent { id: 2, time: Time(20), value: 2 });
    store.process_until(Time(40), &calls).unwrap();

    let before_insert = calls.get();
    let inserted = store.insert(TestEvent { id: 3, time: Time(inserted_time), value: 4 });
    let after_insert = calls.get();
    let queried = store.query(Time(40), &calls).unwrap();
    store.process_until(Time(40), &calls).unwrap();
    let processed = store.query(Time(40), &calls).unwrap();
    let before_repeat = calls.get();
    store.process_until(Time(40), &calls).unwrap();
    let after_repeat = calls.get();

    assert_eq!(inserted, expected_insert);
    assert_eq!(after_insert, before_insert);
    assert_eq!(queried.sum, expected_sum);
    assert_eq!(processed.sum, expected_sum);
    assert_eq!(after_repeat, before_repeat);
}

#[test]
fn duplicate_does_not_invalidate_completed_work() {
    let expected_insert = Insert::Duplicate;
    let expected_sum = 3;

    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), 1);
    let calls = Cell::new(0);
    store.insert(TestEvent { id: 1, time: Time(10), value: expected_sum });
    store.process_until(Time(20), &calls).unwrap();

    let before = calls.get();
    let inserted = store.insert(TestEvent { id: 1, time: Time(0), value: 99 });
    store.process_until(Time(20), &calls).unwrap();
    let actual = store.query(Time(20), &calls).unwrap();
    let after = calls.get();

    assert_eq!(inserted, expected_insert);
    assert_eq!(actual.sum, expected_sum);
    assert_eq!(after, before);
}

#[rstest::rstest]
#[case::sparse(100, 90, 10)]
#[case::unbounded(0, 90, 10)]
#[case::dense(1, 90, 90)]
#[case::same_time(100, 100, 10)]
#[case::initial_timestamp(100, 0, 0)]
fn replay_minimum_includes_reconstructed_prefix(#[case] interval: u64, #[case] inserted: u64, #[case] expected: u64) {
    let target = Time(100);
    let expected_replay = Some(Time(expected));

    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), interval);
    let calls = Cell::new(0);
    store.insert(TestEvent { id: 1, time: Time(10), value: 1 });
    store.insert(TestEvent { id: 2, time: target, value: 1 });
    store.process_until(target, &calls).unwrap();
    let clean = store.earliest_replay_time();
    store.insert(TestEvent { id: 3, time: Time(inserted), value: 1 });
    let before = calls.get();

    let actual = store.earliest_replay_time();
    let after = calls.get();
    store.process_until(target, &calls).unwrap();
    let completed = store.earliest_replay_time();

    assert_eq!(clean, None);
    assert_eq!(actual, expected_replay);
    assert_eq!(after, before);
    assert_eq!(completed, None);
}

#[test]
fn completed_horizon_bucket_does_not_pin_future_replay() {
    let expected_replay = Some(Time(100));

    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), 100);
    let calls = Cell::new(0);
    store.insert(TestEvent { id: 1, time: Time(0), value: 1 });
    store.process_until(Time(0), &calls).unwrap();
    store.insert(TestEvent { id: 2, time: Time(100), value: 1 });

    let actual = store.earliest_replay_time();

    assert_eq!(actual, expected_replay);
}

#[rstest::rstest]
#[case::earliest_first([10, 15])]
#[case::earliest_last([15, 10])]
fn multiple_insertions_preserve_the_earliest_pending_work(#[case] times: [u64; 2]) {
    let expected_partial = 11;
    let expected_final = 66;

    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), 1);
    let calls = Cell::new(0);
    store.insert(TestEvent { id: 1, time: Time(0), value: 1 });
    store.insert(TestEvent { id: 2, time: Time(40), value: 40 });
    store.process_until(Time(50), &calls).unwrap();

    for time in times {
        store.insert(TestEvent { id: time, time: Time(time), value: time });
    }
    store.process_until(Time(12), &calls).unwrap();
    let partial = store.query(Time(12), &calls).unwrap();
    let future_query = store.query(Time(50), &calls).unwrap();
    store.process_until(Time(50), &calls).unwrap();
    let final_state = store.query(Time(50), &calls).unwrap();

    assert_eq!(partial.sum, expected_partial);
    assert_eq!(future_query.sum, expected_final);
    assert_eq!(final_state.sum, expected_final);
}

#[rstest::rstest]
#[case::before_cleanup(false)]
#[case::after_cleanup(true)]
fn horizon_rejects_older_events_but_accepts_boundary(#[case] prune: bool) {
    let expected_rejected = Insert::BeforeHorizon;
    let expected_accepted = Insert::Inserted;
    let expected_sum = 7;

    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), 1);
    let calls = Cell::new(0);
    store.insert(TestEvent { id: 1, time: Time(10), value: 1 });
    store.insert(TestEvent { id: 2, time: Time(20), value: 2 });
    store.process_until(Time(30), &calls).unwrap();
    store.forward(Time(20), &calls, |_, _, _| {}).unwrap();
    if prune {
        store.prune();
    }

    let rejected = store.insert(TestEvent { id: 3, time: Time(19), value: 99 });
    let accepted = store.insert(TestEvent { id: 3, time: Time(20), value: 4 });
    store.process_until(Time(30), &calls).unwrap();
    let actual = store.query(Time(30), &calls).unwrap();

    assert_eq!(rejected, expected_rejected);
    assert_eq!(accepted, expected_accepted);
    assert_eq!(actual.sum, expected_sum);
}

#[rstest::rstest]
#[case::unbounded(0)]
#[case::complete_intervals(1)]
#[case::partial_interval(2)]
#[case::sparse_checkpoints(100)]
fn deferred_pruning_preserves_retained_queries(#[case] interval: u64) {
    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), interval);
    let calls = Cell::new(0);
    store.insert(TestEvent { id: 1, time: Time(10), value: 1 });
    store.process_until(Time(10), &calls).unwrap();
    store.forward(Time(20), &calls, |_, _, _| {}).unwrap();

    for (id, time, value) in [(2, 30, 2), (3, 40, 4)] {
        store.insert(TestEvent { id, time: Time(time), value });
        store.process_until(Time(time), &calls).unwrap();
    }
    for (time, expected) in [(20, 1), (30, 3), (40, 7)] {
        assert_eq!(store.query(Time(time), &calls).unwrap().sum, expected);
    }

    store.prune();
    store.prune();

    for (time, expected) in [(20, 1), (30, 3), (40, 7)] {
        assert_eq!(store.query(Time(time), &calls).unwrap().sum, expected);
    }
}

#[rstest::rstest]
#[case::unbounded(0)]
#[case::complete_intervals(1)]
#[case::partial_interval(2)]
#[case::sparse_checkpoints(100)]
fn boundary_inputs_survive_replay_before_each_prune(#[case] interval: u64) {
    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), interval);
    let calls = Cell::new(0);
    store.insert(TestEvent { id: 1, time: Time(10), value: 1 });
    store.process_until(Time(10), &calls).unwrap();

    for (horizon, boundary_id, boundary_value, later_id, later_time, later_value, boundary_sum, total) in
        [(20, 2, 2, 3, 30, 4, 3, 7), (40, 4, 8, 5, 50, 16, 15, 31)]
    {
        store.forward(Time(horizon), &calls, |_, _, _| {}).unwrap();
        store.insert(TestEvent { id: later_id, time: Time(later_time), value: later_value });
        store.process_until(Time(later_time), &calls).unwrap();
        store.insert(TestEvent { id: boundary_id, time: Time(horizon), value: boundary_value });
        store.process_until(Time(later_time), &calls).unwrap();
        store.prune();

        assert_eq!(store.query(Time(horizon), &calls).unwrap().sum, boundary_sum);
        assert_eq!(store.query(Time(later_time), &calls).unwrap().sum, total);
    }
}

#[cfg(feature = "counters")]
#[rstest::rstest]
#[case::every_event(1, 3)]
#[case::partial_intervals(3, 5)]
#[case::unbounded(0, 5)]
fn counted_replay_measures_each_operation_and_rebuilt_prefix(#[case] interval: u64, #[case] late_count: u64) {
    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), interval);
    let calls = Cell::new(0);
    for id in 1..=4 {
        store.insert(TestEvent { id, time: Time(id * 10), value: id });
    }
    assert_eq!(store.process_until_counted(Time(0), &calls), Ok(0));
    assert_eq!(store.process_until_counted(Time(20), &calls), Ok(2));
    assert_eq!(store.query(Time(40), &calls).unwrap().sum, 10);
    assert_eq!(store.process_until_counted(Time(40), &calls), Ok(2));
    assert_eq!(store.process_until_counted(Time(40), &calls), Ok(0));
    assert_eq!(store.insert(TestEvent { id: 1, time: Time(5), value: 99 }), Insert::Duplicate);
    assert_eq!(store.process_until_counted(Time(40), &calls), Ok(0));

    assert_eq!(store.insert(TestEvent { id: 5, time: Time(25), value: 5 }), Insert::Inserted);
    assert_eq!(store.query(Time(40), &calls).unwrap().sum, 15);
    assert_eq!(store.process_until_counted(Time(40), &calls), Ok(late_count));
    assert_eq!(store.process_until_counted(Time(40), &calls), Ok(0));
    assert_eq!(store.query(Time(40), &calls).unwrap().sum, 15);
}

#[cfg(feature = "counters")]
#[rstest::rstest]
#[case::deferred_pruning(false)]
#[case::physical_pruning(true)]
fn counted_replay_excludes_query_and_forwarding_work(#[case] prune: bool) {
    let mut store = SnapshotStore::new(TestEventStore::default(), TestSnapshot::default(), 1);
    let calls = Cell::new(0);
    for id in 1..=4 {
        store.insert(TestEvent { id, time: Time(id * 10), value: id });
    }
    assert_eq!(store.query(Time(40), &calls).unwrap().sum, 10);
    store.forward(Time(30), &calls, |_, _, _| {}).unwrap();
    if prune {
        store.prune();
        store.prune();
    }
    assert!(store.process_until_counted(Time(29), &calls).is_err());
    assert_eq!(store.process_until_counted(Time(40), &calls), Ok(2));
    assert_eq!(store.process_until_counted(Time(40), &calls), Ok(0));
    assert_eq!(store.query(Time(40), &calls).unwrap().sum, 10);
}
