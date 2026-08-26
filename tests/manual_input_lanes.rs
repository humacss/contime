use contime::{Input, InputBatch, InputLanes, Snapshot, SnapshotLanes};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManualInput {
    id: u128,
    time: i64,
}

impl Input for ManualInput {
    type Time = i64;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        24
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ManualSnapshot {
    id: u128,
    time: i64,
}

impl Snapshot for ManualSnapshot {
    type Time = i64;
    type Input = ManualInput;

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
        24
    }
}

impl SnapshotLanes for ManualSnapshot {
    fn materialize(_snapshot_id: u128, _input: &Self::Input) -> Option<Self> {
        None
    }

    fn lane_index(&self) -> usize {
        0
    }

    fn input_lane_index(_snapshot_id: u128, _input: &Self::Input) -> Option<usize> {
        None
    }
}

impl InputLanes<ManualSnapshot> for ManualInput {
    fn visit_snapshot_ids<F>(&self, _visit: &mut F)
    where
        F: FnMut(u128),
    {
    }

    fn is_event(&self) -> bool {
        false
    }

    fn apply_events(_snapshot: &mut ManualSnapshot, _batch: InputBatch<'_, Self>, _history_input_count: u64) {}
}

#[test]
fn manual_input_lanes_keep_a_zero_allocation_default() {
    let input = ManualInput { id: 1, time: 10 };

    assert_eq!(<ManualInput as InputLanes<ManualSnapshot>>::conservative_allocation_size(&input), 0);
}
