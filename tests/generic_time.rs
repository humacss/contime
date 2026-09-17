use std::ops::{Add, Sub};

use contime::{ApplyBatch, ApplyEvents, ContimeEvent, ContimeSnapshot, Event, Input, Snapshot, SnapshotEvent};

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompositeTime {
    major: i64,
    minor: u64,
}

impl CompositeTime {
    fn new(major: i64, minor: u64) -> Self {
        Self { major, minor }
    }
}

impl Add for CompositeTime {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.major + rhs.major, 0)
    }
}

impl Sub for CompositeTime {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.major - rhs.major, 0)
    }
}

impl contime::ContimeTime for CompositeTime {
    fn saturating_sub(self, rhs: Self) -> Self {
        Self::new(self.major.saturating_sub(rhs.major), 0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositeEvent {
    id: u128,
    snapshot_id: u128,
    time: CompositeTime,
    value: i32,
}

impl Input for CompositeEvent {
    type Time = CompositeTime;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time.clone()
    }

    fn conservative_size(&self) -> u64 {
        52
    }
}

impl Event for CompositeEvent {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompositeSnapshot {
    id: u128,
    time: CompositeTime,
    values: Vec<i32>,
}

impl Snapshot for CompositeSnapshot {
    type Time = CompositeTime;
    type Input = CompositeEvent;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time.clone()
    }

    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        48 + self.values.len() as u64 * 4
    }
}

impl SnapshotEvent<CompositeSnapshot> for CompositeEvent {
    fn snapshot_id(&self) -> u128 {
        self.snapshot_id
    }

    fn set_snapshot_identity(&self, snapshot: &mut CompositeSnapshot) {
        snapshot.id = self.snapshot_id;
    }
}

impl ApplyEvents<CompositeEvent> for CompositeSnapshot {
    fn apply_events(&mut self, batch: ApplyBatch<'_, CompositeEvent>) {
        self.values.extend(batch.events.iter().map(|event| event.value));
    }
}

contime::lanes! {
    mod composite_lanes;
    time CompositeTime;
    snapshots [CompositeSnapshot];
    routes [
        CompositeEvent => [CompositeSnapshot],
    ];
}

fn event(id: u128, major: i64, minor: u64, value: i32) -> CompositeEvent {
    CompositeEvent { id, snapshot_id: 1, time: CompositeTime::new(major, minor), value }
}

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.id, time = self.time.clone(), time_type = CompositeTime, bytes = 52)]
pub struct DerivedCompositeEvent {
    id: u128,
    snapshot_id: u128,
    time: CompositeTime,
    value: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [DerivedCompositeEvent],
    id = [snapshot_id],
    time_type = CompositeTime,
    bytes = 52,
    apply = {
        for event in batch.events {
            match event {
                DerivedCompositeSnapshotEvent::DerivedCompositeEvent(event) => self.values.push(event.value),
            }
        }
    }
)]
pub struct DerivedCompositeSnapshot {
    snapshot_id: u128,
    time: CompositeTime,
    values: Vec<i32>,
}

contime::lanes! {
    mod derived_composite_lanes;
    time CompositeTime;
    snapshots [DerivedCompositeSnapshot];
    routes [
        DerivedCompositeEvent => [DerivedCompositeSnapshot],
    ];
}

#[test]
fn composite_time_orders_minor_values_during_replay() {
    let contime = composite_lanes::Contime::new(1, 1_000_000);

    contime.apply([event(2, 10, 2, 2)].map(Into::into)).expect("later composite time should apply provisionally");
    contime.apply([event(1, 10, 1, 1)].map(Into::into)).expect("earlier composite time should trigger ordered replay");

    let snapshot: CompositeSnapshot = contime
        .query_at(CompositeTime::new(10, 2), &[1])
        .expect("composite-time query should succeed")
        .pop()
        .flatten()
        .expect("snapshot should exist")
        .into();

    assert_eq!(snapshot.values, vec![1, 2], "replay should apply lower composite times before higher composite times");
    assert_eq!(snapshot.time, CompositeTime::new(10, 2), "snapshot time should retain the complete ordered time");
}

#[test]
fn composite_time_query_can_stop_between_minor_values() {
    let contime = composite_lanes::Contime::new(1, 1_000_000);
    contime.apply([event(1, 10, 1, 1), event(2, 10, 2, 2)].map(Into::into)).expect("composite-time events should apply");

    let snapshot: CompositeSnapshot = contime
        .query_at(CompositeTime::new(10, 1), &[1])
        .expect("composite-time query should succeed")
        .pop()
        .flatten()
        .expect("snapshot should exist")
        .into();

    assert_eq!(snapshot.values, vec![1], "query should exclude values after its complete ordered time");
}

#[test]
fn composite_time_horizon_uses_time_arithmetic() {
    let contime = composite_lanes::Contime::with_history_horizon(1, 1_000_000, CompositeTime::new(5, 99));
    contime.apply([event(1, 5, 1, 1), event(2, 10, 1, 2)].map(Into::into)).expect("events inside the initial horizon should apply");

    contime.advance_to(CompositeTime::new(10, 7)).expect("composite time should advance");

    let rejections = contime.apply([event(3, 4, 99, 3)].map(Into::into)).expect("event before the arithmetic horizon should be reported");
    assert_eq!(
        rejections,
        vec![contime::EventRejection::new(3, contime::EventRejectionReason::BeforeHistoryHorizon)],
        "horizon subtraction should use the concrete time implementation and reset minor components"
    );
}

#[test]
fn derives_and_lanes_accept_an_explicit_composite_time_type() {
    let contime = derived_composite_lanes::Contime::new(1, 1_000_000);
    contime
        .apply([DerivedCompositeEvent { id: 1, snapshot_id: 7, time: CompositeTime::new(10, 3), value: 42 }].map(Into::into))
        .expect("derived composite-time event should apply");

    let snapshot: DerivedCompositeSnapshot = contime
        .query_at(CompositeTime::new(10, 3), &[7])
        .expect("derived composite-time snapshot should be queryable")
        .pop()
        .flatten()
        .expect("derived snapshot should exist")
        .into();

    assert_eq!(snapshot.values, vec![42], "derived lane should preserve composite-time application semantics");
    assert_eq!(snapshot.time, CompositeTime::new(10, 3), "derived snapshot should retain its complete ordered time");
}
