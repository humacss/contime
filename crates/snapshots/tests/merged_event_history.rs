use contime_snapshots::{Apply, Checkpoint, Event, EventStore, Snapshot, SnapshotStore};
use std::collections::{btree_map, vec_deque, BTreeMap, VecDeque};
use std::iter::Peekable;

type Time = i64;

struct TestEvent(Time, i32);
struct TestEventStore {
    ordered: VecDeque<TestEvent>,
    late: BTreeMap<Time, TestEvent>,
}
struct Iter<'a> {
    ordered: Peekable<vec_deque::Iter<'a, TestEvent>>,
    late: Peekable<btree_map::Values<'a, Time, TestEvent>>,
    after: Option<Time>,
}
#[derive(Clone)]
struct TestSnapshot(i32, Time);

impl Event for TestEvent {
    type Time = Time;
    fn time(&self) -> Time {
        self.0
    }
}
impl<'a> Iterator for Iter<'a> {
    type Item = &'a TestEvent;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let event = match (self.ordered.peek(), self.late.peek()) {
                (Some(left), Some(right)) if left.0 <= right.0 => self.ordered.next(),
                (Some(_), Some(_)) | (None, Some(_)) => self.late.next(),
                (Some(_), None) => self.ordered.next(),
                (None, None) => None,
            }?;
            if self.after.is_none_or(|time| event.0 > time) {
                return Some(event);
            }
        }
    }
}
impl EventStore for TestEventStore {
    type Time = Time;
    type Event = TestEvent;
    type Iter<'a> = Iter<'a>;
    fn iter_after(&self, boundary: Option<&Time>) -> Iter<'_> {
        Iter { ordered: self.ordered.iter().peekable(), late: self.late.values().peekable(), after: boundary.copied() }
    }
    fn prune_before(&mut self, horizon: &Time) {
        self.ordered.retain(|event| event.0 >= *horizon);
        self.late.retain(|time, _| time >= horizon);
    }
}
impl Snapshot for TestSnapshot {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.1
    }
    fn set_time(&mut self, time: Time) {
        self.1 = time;
    }
}
impl Apply<Checkpoint<TestSnapshot>, i32> for TestEvent {
    fn apply<'a>(state: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, context: &i32)
    where
        Self: 'a,
    {
        state.snapshot.0 += events.map(|event| event.1).sum::<i32>() + context;
    }
}

#[rstest::rstest]
#[case::one(1)]
#[case::two(2)]
#[case::unbounded(0)]
fn happy(#[case] checkpoint_interval: u64) {
    let expected_sum = 30;
    let expected_time: Time = 20;

    let initial_time: Time = 0;
    let initial_sum = 0;
    let first_event_time: Time = 10;
    let context = 10;
    let history = TestEventStore {
        ordered: VecDeque::from([TestEvent(first_event_time, 2), TestEvent(expected_time, 5)]),
        late: BTreeMap::from([(first_event_time, TestEvent(first_event_time, 3))]),
    };
    let mut snapshots = SnapshotStore::new(history, TestSnapshot(initial_sum, initial_time), checkpoint_interval);

    snapshots.process_until(expected_time, &context).unwrap();
    let actual = snapshots.query(expected_time, &context).unwrap();
    let actual_sum = actual.0;
    let actual_time = *actual.time();

    assert_eq!(actual_sum, expected_sum);
    assert_eq!(actual_time, expected_time);
}
