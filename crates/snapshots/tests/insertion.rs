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
    fn apply<'a>(checkpoint: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, calls: &Cell<usize>) {
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
