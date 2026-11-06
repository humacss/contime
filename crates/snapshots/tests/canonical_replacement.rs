use contime_snapshots::{Apply, BatchLookahead, Checkpoint, Event, EventStore, Snapshot, SnapshotStore};
#[derive(Clone)]
struct Record {
    time: i64,
    value: i64,
}
impl Event for Record {
    type Time = i64;
    fn time(&self) -> i64 {
        self.time
    }
}
#[derive(Default)]
struct History(Vec<Record>);
impl EventStore for History {
    type Time = i64;
    type Event = Record;
    type Iter<'a> = Box<dyn Iterator<Item = &'a Record> + 'a>;
    fn iter_after(&self, after: Option<&i64>) -> Self::Iter<'_> {
        {
            let after = after.copied();
            Box::new(self.0.iter().filter(move |event| after.is_none_or(|time| event.time > time)))
        }
    }
    fn prune_before(&mut self, horizon: &i64) {
        self.0.retain(|event| event.time >= *horizon);
    }
}
#[derive(Clone)]
struct State {
    time: i64,
    sum: i64,
}
impl Snapshot for State {
    type Time = i64;
    fn time(&self) -> &i64 {
        &self.time
    }
    fn set_time(&mut self, time: i64) {
        self.time = time;
    }
}
impl Apply<Checkpoint<State>, ()> for Record {
    fn apply<'a>(state: &mut Checkpoint<State>, events: impl Iterator<Item = &'a Self>, _: BatchLookahead<'_, i64>, _: &())
    where
        Self: 'a,
    {
        state.snapshot.sum += events.map(|event| event.value).sum::<i64>();
    }
}
#[test]
fn moving_a_canonical_event_reconstructs_from_the_earliest_affected_time() {
    let mut store = SnapshotStore::new(History(vec![Record { time: 10, value: 5 }]), State { time: 0, sum: 0 }, 1);
    store.process_until(30, &()).unwrap();
    assert_eq!(store.query(15, &()).unwrap().sum, 5);
    store
        .edit_history(10, |history| {
            history.0[0].time = 20;
            true
        })
        .unwrap();
    assert_eq!(store.query(15, &()).unwrap().sum, 0);
    assert_eq!(store.query(20, &()).unwrap().sum, 5);
    store.process_until(30, &()).unwrap();
    store
        .edit_history(10, |history| {
            history.0[0].time = 10;
            true
        })
        .unwrap();
    assert_eq!(store.query(15, &()).unwrap().sum, 5);
}
#[test]
fn editing_before_the_horizon_never_calls_the_policy() {
    let mut store = SnapshotStore::new(History::default(), State { time: 10, sum: 0 }, 1);
    let mut called = false;
    assert!(store
        .edit_history(9, |_| {
            called = true;
            true
        })
        .is_err());
    assert!(!called);
}
