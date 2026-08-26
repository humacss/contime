use std::mem::size_of;

use contime::{Input, InputRoute, Marker, SnapshotBatchBenchmark, TestEvent, TestSnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteMarker {
    event_id: u128,
    time: i64,
    snapshot_ids: Vec<u128>,
}

impl Input for RouteMarker {
    type Time = i64;

    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        size_of::<Self>() as u64 + (self.snapshot_ids.capacity() * size_of::<u128>()) as u64
    }
}

impl Marker for RouteMarker {}

impl InputRoute for RouteMarker {
    fn visit_snapshot_ids<F>(&self, visit: &mut F)
    where
        F: FnMut(u128),
    {
        self.snapshot_ids.iter().copied().for_each(visit);
    }
}

contime::lanes! {
    mod batching_lanes;
    snapshots [TestSnapshot];
    markers [RouteMarker];
    routes [TestEvent => [TestSnapshot]];
}

fn marker<const N: usize>(event_id: u128, snapshot_ids: [u128; N]) -> batching_lanes::InputLanes {
    RouteMarker { event_id, time: 10, snapshot_ids: snapshot_ids.into_iter().collect() }.into()
}

#[test]
fn api_grouping_preserves_first_snapshot_and_per_snapshot_input_order() {
    let grouped = SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([
        marker(1, [7, 3]),
        marker(2, [3, 9]),
        marker(3, [7]),
    ]);

    assert_eq!(grouped, vec![(7, vec![1, 3]), (3, vec![1, 2]), (9, vec![2])]);
}

#[test]
fn api_grouping_discards_unrouted_inputs() {
    let grouped =
        SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([marker(1, []), marker(2, [5])]);

    assert_eq!(grouped, vec![(5, vec![2])]);
}

#[test]
fn routed_event_accounts_retained_allocation_and_snapshot_bytes_once() {
    let total = SnapshotBatchBenchmark::total_conservative_bytes::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([
        TestEvent::Positive(7, 10, 1, 1).into(),
    ]);

    // 42 retained event + 8 apply allocation + 32 retained identity
    // + 28 clean snapshot + 32 complete checkpoint key + 8 history count.
    assert_eq!(150, total);
}
