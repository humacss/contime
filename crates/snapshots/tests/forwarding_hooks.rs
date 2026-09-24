use contime_snapshots::{Apply, Checkpoint, Event, EventStore, Snapshot, SnapshotStore, Timestamp};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Time(u64);
#[derive(Clone, Debug, PartialEq, Eq)]
struct TestSnapshot {
    time: Time,
    compacted_sum: u64,
    recent: Vec<(Time, u64)>,
}
struct TestEvent(Time, u64);
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
impl Apply<Checkpoint<TestSnapshot>> for TestEvent {
    fn apply<'a>(checkpoint: &mut Checkpoint<TestSnapshot>, events: impl Iterator<Item = &'a Self>, _: &()) {
        checkpoint.snapshot.recent.extend(events.map(|event| (event.0, event.1)));
    }
}
fn compact(snapshot: &mut TestSnapshot, through: &Time, _: &()) {
    let end = snapshot.recent.partition_point(|(time, _)| time <= through);
    snapshot.compacted_sum += snapshot.recent.drain(..end).map(|(_, value)| value).sum::<u64>();
}

#[rstest::rstest]
#[case::gap_small_interval(20, 1, 3, vec![(Time(20), 4)])]
#[case::gap_large_interval(20, 100, 3, vec![(Time(20), 4)])]
#[case::event_at_predecessor(21, 1, 7, vec![])]
fn happy(#[case] horizon: u64, #[case] interval: u64, #[case] compacted_sum: u64, #[case] recent: Vec<(Time, u64)>) {
    let expected_at_horizon = TestSnapshot { time: Time(20), compacted_sum, recent: recent.clone() };
    let mut expected_replayed = TestSnapshot { time: Time(30), compacted_sum, recent };
    expected_replayed.recent.push((Time(30), 8));
    let expected_final = TestSnapshot { time: Time(39), compacted_sum: 15, recent: vec![] };

    let initial = TestSnapshot { time: Time(0), compacted_sum: 0, recent: vec![] };
    let events = TestEventStore(vec![TestEvent(Time(10), 3), TestEvent(Time(20), 4), TestEvent(Time(30), 8)]);
    let mut snapshots = SnapshotStore::new(events, initial, interval);

    snapshots.forward(Time(horizon), &(), compact).unwrap();
    let before_prune = snapshots.query(Time(horizon), &()).unwrap();
    snapshots.prune();
    let after_prune = snapshots.query(Time(horizon), &()).unwrap();
    snapshots.replay(Time(30), &()).unwrap();
    let replayed = snapshots.query(Time(30), &()).unwrap();
    snapshots.forward(Time(40), &(), compact).unwrap();
    snapshots.prune();
    let forwarded_again = snapshots.query(Time(40), &()).unwrap();

    assert_eq!(*before_prune, expected_at_horizon);
    assert_eq!(*after_prune, expected_at_horizon);
    assert_eq!(*replayed, expected_replayed);
    assert_eq!(*forwarded_again, expected_final);
}

#[test]
fn hook_mutations_persist_when_no_events_are_applied() {
    let expected = TestSnapshot { time: Time(19), compacted_sum: 3, recent: vec![] };

    let initial = TestSnapshot { time: Time(10), compacted_sum: 0, recent: vec![(Time(10), 3)] };
    let mut snapshots = SnapshotStore::new(TestEventStore(vec![]), initial, 1);

    snapshots.forward(Time(20), &(), compact).unwrap();
    snapshots.prune();
    let actual = snapshots.query(Time(20), &()).unwrap();

    assert_eq!(*actual, expected);
}
