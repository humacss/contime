use contime::{
    AfterApplyEvents, ApplyBatch, ApplyEvents, Event, QueryResult, Snapshot, SnapshotEvent, TestEvent, TestSnapshot, TestSnapshotContime,
};

#[test]
fn scheduled_event_does_not_apply_before_advance() {
    let contime = TestSnapshotContime::new(1, 10_000);

    contime.schedule_event(TestEvent::Positive(1, 5, 10, 3)).expect("event should schedule");

    assert!(matches!(contime.query_at(6, 1).unwrap().wait().unwrap(), QueryResult::NotFound));
}

#[test]
fn advance_to_due_time_releases_and_applies_scheduled_event() {
    let contime = TestSnapshotContime::new(1, 10_000);

    contime
        .send_scheduled_event(TestEvent::Positive(1, 5, 10, 3))
        .expect("event should schedule")
        .wait()
        .expect("schedule should complete");
    contime.advance_to(5).expect("time should advance");

    let (snapshot, _) = contime.at::<TestSnapshot>(6, 1).expect("snapshot should exist");
    assert_eq!(snapshot.sum, 3);
}

#[test]
fn due_events_apply_in_time_then_event_id_order() {
    let contime = TestSnapshotContime::new(1, 10_000);

    contime.schedule_event(TestEvent::Positive(1, 7, 30, 100)).unwrap();
    contime.schedule_event(TestEvent::Positive(1, 5, 20, 10)).unwrap();
    contime.schedule_event(TestEvent::Positive(1, 5, 10, 1)).unwrap();

    contime.advance_to(7).expect("time should advance");

    let (snapshot, _) = contime.at::<TestSnapshot>(8, 1).expect("snapshot should exist");
    assert_eq!(snapshot.items, vec![1, 10, 100]);
    assert_eq!(snapshot.sum, 111);
}

#[test]
fn scheduled_event_matches_immediate_event_state() {
    let scheduled = TestSnapshotContime::new(1, 10_000);
    let immediate = TestSnapshotContime::new(1, 10_000);

    scheduled.schedule_event(TestEvent::Positive(1, 5, 10, 3)).unwrap();
    scheduled.advance_to(5).expect("time should advance");
    immediate.apply_event(TestEvent::Positive(1, 5, 10, 3)).unwrap();

    let (scheduled_snapshot, _) = scheduled.at::<TestSnapshot>(6, 1).unwrap();
    let (immediate_snapshot, _) = immediate.at::<TestSnapshot>(6, 1).unwrap();
    assert_eq!(scheduled_snapshot, immediate_snapshot);
}

#[test]
fn cancellation_before_due_prevents_application() {
    let contime = TestSnapshotContime::new(1, 10_000);

    contime.schedule_event(TestEvent::Positive(1, 5, 10, 3)).unwrap();
    contime.cancel_scheduled_event(10, 5).expect("cancel should enqueue").wait().expect("cancel should complete");
    contime.advance_to(5).expect("time should advance");

    assert!(matches!(contime.query_at(6, 1).unwrap().wait().unwrap(), QueryResult::NotFound));
}

#[test]
fn repeated_schedule_with_same_identity_replaces_payload() {
    let contime = TestSnapshotContime::new(1, 10_000);

    contime.schedule_event(TestEvent::Positive(1, 5, 10, 3)).unwrap();
    contime.schedule_event(TestEvent::Positive(1, 5, 10, 9)).unwrap();
    contime.advance_to(5).expect("time should advance");

    let (snapshot, _) = contime.at::<TestSnapshot>(6, 1).expect("snapshot should exist");
    assert_eq!(snapshot.sum, 9);
}

#[test]
fn keyed_schedule_replaces_older_future_event_for_same_snapshot() {
    let contime = TestSnapshotContime::new(1, 10_000);

    contime.schedule_keyed_event(77, TestEvent::Positive(1, 5, 10, 3)).unwrap();
    contime.schedule_keyed_event(77, TestEvent::Positive(1, 8, 20, 9)).unwrap();
    contime.advance_to(8).expect("time should advance");

    let (snapshot, _) = contime.at::<TestSnapshot>(9, 1).expect("snapshot should exist");
    assert_eq!(snapshot.items, vec![9]);
    assert_eq!(snapshot.sum, 9);
}

#[test]
fn different_schedule_keys_do_not_replace_each_other() {
    let contime = TestSnapshotContime::new(1, 10_000);

    contime.schedule_keyed_event(77, TestEvent::Positive(1, 5, 10, 3)).unwrap();
    contime.schedule_keyed_event(88, TestEvent::Positive(1, 5, 20, 9)).unwrap();
    contime.advance_to(5).expect("time should advance");

    let (snapshot, _) = contime.at::<TestSnapshot>(6, 1).expect("snapshot should exist");
    assert_eq!(snapshot.items, vec![3, 9]);
    assert_eq!(snapshot.sum, 12);
}

#[test]
fn released_scheduled_events_apply_before_later_inbound_events() {
    let contime = TestSnapshotContime::new(1, 10_000);

    contime.schedule_event(TestEvent::Positive(1, 5, 10, 5)).unwrap();
    let mut advance = contime.send_advance_to(5).expect("advance should enqueue");
    contime.send_event(TestEvent::Positive(1, 6, 20, 7)).expect("later event should enqueue").wait().expect("later event should apply");
    advance.try_recv().expect("advance should not fail");

    let (snapshot, _) = contime.at::<TestSnapshot>(7, 1).expect("snapshot should exist");
    assert_eq!(snapshot.items, vec![5, 7]);
    assert_eq!(snapshot.sum, 12);
}

#[test]
fn applied_scheduled_event_replays_after_older_event_arrives() {
    let contime = TestSnapshotContime::new(1, 10_000);

    contime.apply_event(TestEvent::Positive(1, 1, 1, 1)).unwrap();
    contime.schedule_event(TestEvent::Positive(1, 1001, 1001, 1000)).unwrap();
    contime.advance_to(1100).expect("scheduled event should be released");

    let (before, _) = contime.at::<TestSnapshot>(1100, 1).expect("snapshot should exist");
    assert_eq!(before.items, vec![1, 1000]);
    assert_eq!(before.sum, 1001);

    contime.apply_event(TestEvent::Positive(1, 876, 876, 10)).unwrap();

    let (after, _) = contime.at::<TestSnapshot>(1100, 1).expect("snapshot should exist");
    assert_eq!(after.items, vec![1, 10, 1000]);
    assert_eq!(after.sum, 1011);
}

#[test]
fn multi_snapshot_scheduled_event_applies_to_all_targets() {
    let contime = multi::Contime::new(2, 10_000);

    contime.schedule_event(MultiEvent { event_id: 10, time: 5, left_id: 1, right_id: 2, value: 7 }).expect("event should schedule");
    contime.advance_to(5).expect("time should advance");

    let mut lanes = contime.many_at(6, &[1, 2]).expect("snapshots should query");
    let right = RightAt::try_from(lanes.pop().expect("right slot").expect("right lane")).expect("right snapshot");
    let left = LeftAt::try_from(lanes.pop().expect("left slot").expect("left lane")).expect("left snapshot");
    assert_eq!(left.value, 7);
    assert_eq!(right.value, 7);
}

#[test]
fn send_advance_to_reserves_current_time_without_waiting() {
    let contime = TestSnapshotContime::new(1, 10_000);

    let _first = contime.send_advance_to(5).expect("first advance should enqueue");
    let second = contime.send_advance_to(7).expect("second advance should enqueue");

    assert_eq!(contime.current_time(), 7);
    second.wait().expect("second advance should complete");
}

#[test]
fn immediate_event_routes_by_snapshot_event_id_not_seed_snapshot_id() {
    let contime = raw_route::Contime::new(1, 10_000);

    contime.apply_event(RawRouteEvent { event_id: 10, time: 5, target_snapshot_id: 123, value: 7 }).expect("event should apply");

    let routed = contime
        .many_at(6, &[123])
        .expect("target snapshot should query")
        .pop()
        .flatten()
        .and_then(|lane| RawRouteAt::try_from(lane).ok())
        .expect("target snapshot should exist");
    assert_eq!(routed.snapshot_id, 123);
    assert_eq!(routed.applied_values, vec![7]);
    assert!(
        contime.many_at(6, &[0]).expect("default snapshot should query").pop().flatten().is_none(),
        "seed snapshot identity must not become the routed lane"
    );
}

#[test]
fn scheduled_event_routes_by_snapshot_event_id_not_seed_snapshot_id() {
    let contime = raw_route::Contime::new(1, 10_000);

    contime
        .schedule_keyed_event(55, RawRouteEvent { event_id: 10, time: 5, target_snapshot_id: 123, value: 7 })
        .expect("event should schedule");
    contime.advance_to(5).expect("scheduled event should apply");

    let routed = contime
        .many_at(6, &[123])
        .expect("target snapshot should query")
        .pop()
        .flatten()
        .and_then(|lane| RawRouteAt::try_from(lane).ok())
        .expect("target snapshot should exist");
    assert_eq!(routed.snapshot_id, 123);
    assert_eq!(routed.applied_values, vec![7]);
    assert!(
        contime.many_at(6, &[0]).expect("default snapshot should query").pop().flatten().is_none(),
        "seed snapshot identity must not become the routed lane"
    );
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LeftAt {
    id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RightAt {
    id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MultiEvent {
    event_id: u128,
    time: i64,
    left_id: u128,
    right_id: u128,
    value: i32,
}

impl Snapshot for LeftAt {
    type Event = MultiEvent;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        Self { id: event.left_id, time: event.time, value: 0 }
    }
}

impl Snapshot for RightAt {
    type Event = MultiEvent;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        Self { id: event.right_id, time: event.time, value: 0 }
    }
}

impl Event for MultiEvent {
    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl SnapshotEvent<LeftAt> for MultiEvent {
    fn snapshot_id(&self) -> u128 {
        self.left_id
    }
}

impl SnapshotEvent<RightAt> for MultiEvent {
    fn snapshot_id(&self) -> u128 {
        self.right_id
    }
}

impl ApplyEvents for LeftAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>) {
        if let Some(event) = batch.events.last() {
            self.id = event.left_id;
            self.value = event.value;
        }
        self.time = batch.time;
    }
}

impl ApplyEvents for RightAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>) {
        if let Some(event) = batch.events.last() {
            self.id = event.right_id;
            self.value = event.value;
        }
        self.time = batch.time;
    }
}

impl<C> AfterApplyEvents<C> for LeftAt {}
impl<C> AfterApplyEvents<C> for RightAt {}

contime::contime! {
    mod multi;
    snapshots { LeftAt, RightAt },
    Multi(MultiEvent) => [LeftAt, RightAt],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RawRouteAt {
    snapshot_id: u128,
    time: i64,
    applied_values: Vec<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RawRouteEvent {
    event_id: u128,
    time: i64,
    target_snapshot_id: u128,
    value: i32,
}

impl Snapshot for RawRouteAt {
    type Event = RawRouteEvent;

    fn id(&self) -> u128 {
        self.snapshot_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        (std::mem::size_of::<Self>() + self.applied_values.len() * std::mem::size_of::<i32>()) as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        Self { snapshot_id: 0, time: event.time, applied_values: Vec::new() }
    }
}

impl Event for RawRouteEvent {
    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl SnapshotEvent<RawRouteAt> for RawRouteEvent {
    fn snapshot_id(&self) -> u128 {
        self.target_snapshot_id
    }
}

impl ApplyEvents for RawRouteAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>) {
        self.snapshot_id = batch.snapshot_id;
        for event in batch.events {
            self.applied_values.push(event.value);
        }
        self.time = batch.time;
    }
}

impl<C> AfterApplyEvents<C> for RawRouteAt {}

contime::contime! {
    mod raw_route;
    snapshots { RawRouteAt },
    RawRoute(RawRouteEvent) => [RawRouteAt],
}
