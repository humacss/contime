use contime_snapshots::{Apply, Checkpoint, Event, EventStore, Snapshot, SnapshotStore};
use std::cell::RefCell;

type Time = u64;

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestSnapshot {
    time: Time,
    balance: i64,
}
struct TestEvent(Time, i64);
struct TestEventStore(Vec<TestEvent>);
#[derive(Default)]
struct Context {
    batches: RefCell<Vec<(Time, usize)>>,
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
    }
}
impl Apply<Checkpoint<TestSnapshot>, Context> for TestEvent {
    fn apply<'a>(checkpoint: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, context: &Context) {
        let mut count = 0;
        let mut time = 0;
        for event in events {
            checkpoint.snapshot.balance += event.1;
            time = event.0;
            count += 1;
        }
        context.batches.borrow_mut().push((time, count));
    }
}

#[rstest::rstest]
#[case::zero(0, 13, 0)]
#[case::gap(9, 13, 0)]
#[case::exact(10, 9, 10)]
#[case::future(100, 16, 20)]
fn happy(#[case] target: Time, #[case] expected_balance: i64, #[case] expected_time: Time) {
    let expected = TestSnapshot { time: expected_time, balance: expected_balance };

    let events = TestEventStore(vec![TestEvent(0, 1), TestEvent(0, 2), TestEvent(10, -4), TestEvent(20, 7)]);
    let initial = TestSnapshot { time: 0, balance: 10 };
    let mut store = SnapshotStore::new(events, initial, 1);
    let context = Context::default();

    let actual = store.query(target, &context).unwrap();

    assert_eq!(*actual, expected);
}

#[test]
fn returned_state_and_query_progress_are_not_retained() {
    let expected = TestSnapshot { time: 10, balance: 5 };
    let expected_batches = vec![(10, 1), (10, 1)];

    let events = TestEventStore(vec![TestEvent(10, 5)]);
    let mut store = SnapshotStore::new(events, TestSnapshot { time: 0, balance: 0 }, 1);
    let context = Context::default();

    let mut first = store.query(10, &context).unwrap();
    first.balance = -100;
    let actual = store.query(10, &context).unwrap();

    assert_eq!(*actual, expected);
    assert_eq!(*context.batches.borrow(), expected_batches);
}

#[test]
fn empty_history_preserves_initial_state() {
    let expected = TestSnapshot { time: 0, balance: 42 };

    let mut store = SnapshotStore::new(TestEventStore(vec![]), expected.clone(), 1);
    let context = Context::default();

    let actual = store.query(100, &context).unwrap();

    assert_eq!(*actual, expected);
}
