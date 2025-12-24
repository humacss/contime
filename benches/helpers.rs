use contime::{ApplyEvent, Event, Snapshot};

type EventId = u128;
type SnapshotId = u128;
type Time = i64;

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct BenchSnapshot {
    pub id: u128,
    pub time: i64,

    pub sum: i32,
}

impl Snapshot for BenchSnapshot {
    type Event = BenchEvent;

    fn id(&self) -> u128 {
        self.id
    }
    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BenchEvent {
    Positive(EventId, Time, SnapshotId, u16),
    Negative(EventId, Time, SnapshotId, u16),
}

impl Event for BenchEvent {
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
}

impl ApplyEvent<BenchSnapshot> for BenchEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::Positive(snapshot_id, _time, _event_id, _value) => *snapshot_id,
            Self::Negative(snapshot_id, _time, _event_id, _value) => *snapshot_id,
        }
    }

    fn apply_to(&self, snapshot: &mut BenchSnapshot) {
        if self.snapshot_id() != snapshot.id {
            return;
        }

        match self {
            Self::Positive(_snapshot_id, _time, _event_id, value) => {
                snapshot.sum += *value as i32;
            }
            Self::Negative(_snapshot_id, _time, _event_id, value) => {
                snapshot.sum -= *value as i32;
            }
        }
    }
}
