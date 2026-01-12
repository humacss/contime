use crate::{ApplyEvent, Event, Snapshot};

type EventId = u128;
type SnapshotId = u128;
type Time = i64;

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
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TestEvent {
    Positive(EventId, Time, SnapshotId, u16),
    Negative(EventId, Time, SnapshotId, u16),
}

impl Event for TestEvent {
    fn id(&self) -> u128 {
        match self {
            Self::Positive(_snapshot_id, _time, event_id, _value) => *event_id,
            Self::Negative(_snapshot_id, _time, event_id, _value) => *event_id,
        }
    }
    fn time(&self) -> i64 {
        match self {
            Self::Positive(_snapshot_id, time, _event_id, _value) => *time,
            Self::Negative(_snapshot_id, time, _event_id, _value) => *time,
        }
    }

    fn conservative_size(&self) -> usize { 
        16 + 8 + 16 + 2
    }    
}

impl ApplyEvent<TestSnapshot> for TestEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::Positive(snapshot_id, _time, _event_id, _value) => *snapshot_id,
            Self::Negative(snapshot_id, _time, _event_id, _value) => *snapshot_id,
        }
    }

    fn apply_to(&self, snapshot: &mut TestSnapshot) -> isize {
        match self {
            Self::Positive(_snapshot_id, _time, _event_id, value) => {
                snapshot.items.push(*value as i16);
                snapshot.sum += *value as i32;

                2
            }
            Self::Negative(_snapshot_id, _time, _event_id, value) => {
                snapshot.items.push(-(*value as i16));
                snapshot.sum -= *value as i32;

                2
            }
        }
    }
    
    fn conservative_apply_size_delta(&self) -> isize {
        match self {
            Self::Positive(..) => 2,
            Self::Negative(..) => 2, 
        }
    }
}
