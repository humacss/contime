use crate::{Snapshot, TestEvent, ApplyEvent};

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

    fn conservative_size(&self) -> usize {
        16 + 8 + 4 + (self.items.len() * 2)
    }
    
    fn from_event(event: &Self::Event) -> Self {
        let mut s = Self::default();

        s.id = event.snapshot_id();

        s
    }
}
