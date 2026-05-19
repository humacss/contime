use crate::{AfterApplyEvents, ApplyEvents, Event, Snapshot, SnapshotEvent, TestEvent};

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct TestSnapshot {
    pub id: u128,
    pub time: i64,

    pub items: Vec<i16>,
    pub sum: i32,
}

impl Snapshot for TestSnapshot {
    type Event = TestEvent;

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
        16 + 8 + 4 + (self.items.len() * 2) as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        Self { id: <TestEvent as SnapshotEvent<TestSnapshot>>::snapshot_id(event), time: event.time(), ..Self::default() }
    }
}

impl ApplyEvents for TestSnapshot {
    fn apply_events(&mut self, time: i64, events: &[Self::Event]) {
        for event in events {
            match event {
                TestEvent::Positive(_snapshot_id, _time, _event_id, value) => {
                    self.items.push(*value as i16);
                    self.sum += *value as i32;
                }
                TestEvent::Negative(_snapshot_id, _time, _event_id, value) => {
                    self.items.push(-(*value as i16));
                    self.sum -= *value as i32;
                }
            }
        }
        self.set_time(time);
    }
}

impl<C> AfterApplyEvents<C> for TestSnapshot {}
