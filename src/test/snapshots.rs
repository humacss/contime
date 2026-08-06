use crate::{ApplyBatch, ApplyEvents, Snapshot, SnapshotEvent, SnapshotLanes, TestEvent};

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct TestSnapshot {
    pub id: u128,
    pub time: i64,

    pub items: Vec<i16>,
    pub sum: i32,
}

impl Snapshot for TestSnapshot {
    type Time = i64;
    type Input = TestEvent;

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
}

impl SnapshotLanes for TestSnapshot {
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

impl ApplyEvents<TestEvent> for TestSnapshot {
    fn apply_events(&mut self, batch: ApplyBatch<'_, TestEvent>) {
        self.id = batch.snapshot_id;
        for event in batch.events.iter().copied() {
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
        self.set_time(batch.time);
    }
}
