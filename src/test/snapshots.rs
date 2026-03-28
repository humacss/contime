use crate::{ApplyEvent, Event, Snapshot, TestEvent};

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
        Self { id: event.snapshot_id(), time: event.time(), ..Self::default() }
    }
}
