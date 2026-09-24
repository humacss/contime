//! Processing through inclusive targets retains progress for later calls.
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
#[case::unbounded(0)]
#[case::smaller_than_timestamp_batch(1)]
#[case::larger_than_history(100)]
fn happy(#[case] interval: u64) {
    let expected = TestSnapshot { time: 20, balance: 6 };
    let expected_historical = TestSnapshot { time: 10, balance: -1 };
    let expected_batches = vec![(0, 2), (10, 1), (20, 1)];

    let events = TestEventStore(vec![TestEvent(0, 1), TestEvent(0, 2), TestEvent(10, -4), TestEvent(20, 7)]);
    let mut store = SnapshotStore::new(events, TestSnapshot { time: 0, balance: 0 }, interval);
    let context = Context::default();

    store.process_until(0, &context).unwrap();
    store.process_until(10, &context).unwrap();
    store.process_until(20, &context).unwrap();
    store.process_until(100, &context).unwrap();
    let replay_batches = context.batches.borrow().clone();
    let actual = store.query(20, &context).unwrap();
    let historical = store.query(10, &context).unwrap();

    assert_eq!(*actual, expected);
    assert_eq!(*historical, expected_historical);
    assert_eq!(replay_batches, expected_batches);
}

#[test]
fn target_before_next_event_leaves_it_pending() {
    let expected_before = TestSnapshot { time: 0, balance: 0 };
    let expected_after = TestSnapshot { time: 10, balance: 5 };
    let expected_batches = vec![(10, 1)];

    let events = TestEventStore(vec![TestEvent(10, 5)]);
    let mut store = SnapshotStore::new(events, expected_before.clone(), 1);
    let context = Context::default();

    store.process_until(9, &context).unwrap();
    let before = store.query(9, &context).unwrap();
    store.process_until(10, &context).unwrap();
    let after = store.query(10, &context).unwrap();

    assert_eq!(*before, expected_before);
    assert_eq!(*after, expected_after);
    assert_eq!(*context.batches.borrow(), expected_batches);
}
