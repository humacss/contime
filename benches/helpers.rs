use contime::{ApplyBatch, ApplyEvents, Event, Input, Snapshot, SnapshotEvent, SnapshotLanes};

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
    type Time = i64;
    type Input = BenchEvent;

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
        16 + 8 + 4
    }
}

impl SnapshotLanes for BenchSnapshot {
    fn materialize(snapshot_id: u128, input: &Self::Input) -> Option<Self> {
        if input.snapshot_id() != snapshot_id {
            return None;
        }

        let mut snapshot = Self::default();
        input.set_snapshot_identity(&mut snapshot);
        Some(snapshot)
    }

    fn lane_index(&self) -> usize {
        0
    }

    fn input_lane_index(snapshot_id: u128, input: &Self::Input) -> Option<usize> {
        (input.snapshot_id() == snapshot_id).then_some(0)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BenchEvent {
    Positive(EventId, Time, SnapshotId, u16),
}

impl Input for BenchEvent {
    type Time = i64;

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

impl Event for BenchEvent {}

impl SnapshotEvent<BenchSnapshot> for BenchEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::Positive(snapshot_id, _time, _event_id, _value) => *snapshot_id,
        }
    }

    fn set_snapshot_identity(&self, snapshot: &mut BenchSnapshot) {
        snapshot.id = self.snapshot_id();
    }
}

impl ApplyEvents<BenchEvent> for BenchSnapshot {
    fn apply_events(&mut self, batch: ApplyBatch<'_, BenchEvent>) {
        self.id = batch.snapshot_id;
        for event in batch.events.iter().copied() {
            match event {
                BenchEvent::Positive(_snapshot_id, _time, _event_id, value) => {
                    self.sum += *value as i32;
                }
            }
        }
        self.time = batch.time;
    }
}

contime::lanes! {
    mod bench_lanes;
    snapshots [BenchSnapshot];
    routes [
        BenchEvent => [BenchSnapshot],
    ];
}

#[allow(dead_code)]
pub type BenchContime = bench_lanes::Contime;
