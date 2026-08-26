use contime::{ApplyBatch, ApplyEvents, Contime, Event, Input, Snapshot, SnapshotEvent, SnapshotLanes};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ValueChanged {
    id: u128,
    snapshot_id: u128,
    time: i64,
    value: i64,
}

impl Input for ValueChanged {
    type Time = i64;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        size_of::<Self>() as u64
    }
}

impl Event for ValueChanged {
    fn conservative_allocation_size(&self) -> u64 {
        4 * size_of::<u128>() as u64
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ValueAt {
    id: u128,
    time: i64,
    value: i64,
    retained_input_ids: Vec<u128>,
}

impl Snapshot for ValueAt {
    type Time = i64;
    type Input = ValueChanged;

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
        (size_of::<Self>() + self.retained_input_ids.capacity() * size_of::<u128>()) as u64
    }

    fn compact_before(&mut self, _time: i64) {
        self.retained_input_ids.clear();
        self.retained_input_ids.shrink_to_fit();
    }
}

impl SnapshotLanes for ValueAt {
    fn materialize(snapshot_id: u128, input: &Self::Input) -> Option<Self> {
        (input.snapshot_id == snapshot_id).then_some(Self { id: snapshot_id, ..Self::default() })
    }

    fn lane_index(&self) -> usize {
        0
    }

    fn input_lane_index(snapshot_id: u128, input: &Self::Input) -> Option<usize> {
        (input.snapshot_id == snapshot_id).then_some(0)
    }
}

impl SnapshotEvent<ValueAt> for ValueChanged {
    fn snapshot_id(&self) -> u128 {
        self.snapshot_id
    }

    fn set_snapshot_identity(&self, snapshot: &mut ValueAt) {
        snapshot.id = self.snapshot_id;
    }
}

impl ApplyEvents<ValueChanged> for ValueAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, ValueChanged>) {
        for event in batch.events {
            self.value = event.value;
            self.retained_input_ids.push(event.id);
        }
        self.time = batch.time;
    }
}

#[test]
fn horizon_compaction_preserves_effects_without_retaining_old_input_references() {
    let contime = Contime::<ValueAt, ValueChanged>::with_history_horizon(1, 100_000, 10);
    contime.apply([ValueChanged { id: 11, snapshot_id: 7, time: 20, value: 42 }]).expect("the source event should apply");

    let before = contime.query_at(20, &[7]).expect("the snapshot should be queryable");
    assert_eq!(before[0].as_ref().map(|snapshot| snapshot.retained_input_ids.as_slice()), Some([11].as_slice()));

    contime.advance_to(40).expect("the horizon should advance beyond the source event");

    let compacted = contime.query_at(40, &[7]).expect("the compacted snapshot should remain queryable");
    let compacted = compacted[0].as_ref().expect("the accumulated snapshot effect should remain");
    assert_eq!(compacted.value, 42);
    assert!(compacted.retained_input_ids.is_empty(), "the replay anchor retained a pruned input reference");
}
