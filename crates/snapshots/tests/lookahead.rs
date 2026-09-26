use contime_snapshots::{Apply, BatchLookahead, Checkpoint, Event, EventStore, Snapshot, SnapshotStore};
use std::cell::RefCell;

type Time = u64;

#[derive(Clone)]
struct TestSnapshot {
    time: Time,
}

struct TestEvent(Time);
struct TestEventStore(Vec<TestEvent>);

#[derive(Default)]
struct Context {
    batches: RefCell<Vec<(Time, usize, Option<Time>)>>,
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
    fn apply<'a>(
        _: &mut Checkpoint<TestSnapshot>,
        events: impl Iterator<Item = &'a Self>,
        lookahead: BatchLookahead<'_, Time>,
        context: &Context,
    ) {
        let mut time = None;
        let mut count = 0;
        for event in events {
            time.get_or_insert(event.0);
            count += 1;
        }
        context.batches.borrow_mut().push((time.unwrap(), count, lookahead.next_event_time()));
    }
}

#[test]
fn timestamp_batches_expose_the_next_event_time() {
    let events = TestEventStore(vec![TestEvent(0), TestEvent(0), TestEvent(10), TestEvent(20)]);
    let mut store = SnapshotStore::new(events, TestSnapshot { time: 0 }, 100);
    let context = Context::default();

    store.process_until(20, &context).unwrap();

    assert_eq!(*context.batches.borrow(), vec![(0, 2, Some(10)), (10, 1, Some(20)), (20, 1, None)]);
}
