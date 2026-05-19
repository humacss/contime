use contime::{AfterApplyEvents, ApplyEvents, Event, Snapshot, SnapshotEvent};

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

    fn from_event(event: &<Self as Snapshot>::Event) -> Self {
        Self { id: <BenchEvent as SnapshotEvent<BenchSnapshot>>::snapshot_id(event), ..Self::default() }
    }

    fn conservative_size(&self) -> u64 {
        16 + 8 + 4
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BenchEvent {
    Positive(EventId, Time, SnapshotId, u16),
}

impl Event for BenchEvent {
    fn id(&self) -> u128 {
        match self {
            Self::Positive(_snapshot_id, _time, event_id, _value) => *event_id,
        }
    }
    fn time(&self) -> i64 {
        match self {
            Self::Positive(_snapshot_id, time, _event_id, _value) => *time,
        }
    }

    fn conservative_size(&self) -> u64 {
        16 + 8 + 16 + 2
    }
}

impl SnapshotEvent<BenchSnapshot> for BenchEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::Positive(snapshot_id, _time, _event_id, _value) => *snapshot_id,
        }
    }
}

impl ApplyEvents for BenchSnapshot {
    fn apply_events(&mut self, time: i64, events: &[Self::Event]) {
        for event in events {
            if <BenchEvent as SnapshotEvent<BenchSnapshot>>::snapshot_id(event) != self.id {
                continue;
            }

            match event {
                BenchEvent::Positive(_snapshot_id, _time, _event_id, value) => {
                    self.sum += *value as i32;
                }
            }
        }
        self.time = time;
    }
}

impl<C> AfterApplyEvents<C> for BenchSnapshot {}

contime::contime! {
    mod bench_lanes;
    BenchSnapshot { BenchEvent }
}

#[allow(dead_code)]
pub type BenchContime = bench_lanes::Contime;
