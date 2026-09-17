use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

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

#[derive(Debug)]
pub struct CloneCountedRoute {
    event_id: u128,
    time: i64,
    snapshot_ids: Vec<u128>,
    clones: Arc<AtomicUsize>,
}

impl PartialEq for CloneCountedRoute {
    fn eq(&self, other: &Self) -> bool {
        self.event_id == other.event_id && self.time == other.time && self.snapshot_ids == other.snapshot_ids
    }
}

impl Eq for CloneCountedRoute {}

impl Clone for CloneCountedRoute {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::Relaxed);
        Self { event_id: self.event_id, time: self.time, snapshot_ids: self.snapshot_ids.clone(), clones: Arc::clone(&self.clones) }
    }
}

impl Input for CloneCountedRoute {
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

impl Marker for CloneCountedRoute {}

impl InputRoute for CloneCountedRoute {
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
    markers [RouteMarker, CloneCountedRoute];
    routes [TestEvent => [TestSnapshot]];
}

fn marker<const N: usize>(event_id: u128, snapshot_ids: [u128; N]) -> batching_lanes::InputLanes {
    RouteMarker { event_id, time: 10, snapshot_ids: snapshot_ids.into_iter().collect() }.into()
}

#[test]
fn api_grouping_builds_one_complete_batch_per_snapshot_without_order_semantics() {
    let grouped = SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([
        marker(1, [7, 3]),
        marker(2, [3, 9]),
        marker(3, [7]),
    ]);

    assert_eq!(grouped.get(&7), Some(&vec![1, 3]));
    assert_eq!(grouped.get(&3), Some(&vec![1, 2]));
    assert_eq!(grouped.get(&9), Some(&vec![2]));
    assert_eq!(grouped.len(), 3);
}

#[test]
fn api_grouping_discards_unrouted_inputs() {
    let grouped =
        SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([marker(1, []), marker(2, [5])]);

    assert_eq!(grouped.get(&5), Some(&vec![2]));
    assert_eq!(grouped.len(), 1);
}

#[test]
fn one_route_moves_the_input_without_cloning() {
    let clones = Arc::new(AtomicUsize::new(0));
    let input = CloneCountedRoute { event_id: 1, time: 10, snapshot_ids: vec![7], clones: Arc::clone(&clones) };

    let grouped = SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([input.into()]);

    assert_eq!(grouped.get(&7), Some(&vec![1]));
    assert_eq!(clones.load(Ordering::Relaxed), 0);
}

#[test]
fn three_routes_clone_the_input_exactly_twice() {
    let clones = Arc::new(AtomicUsize::new(0));
    let input = CloneCountedRoute { event_id: 1, time: 10, snapshot_ids: vec![7, 11, 19], clones: Arc::clone(&clones) };

    let grouped = SnapshotBatchBenchmark::group::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([input.into()]);

    assert_eq!(grouped.len(), 3);
    assert_eq!(clones.load(Ordering::Relaxed), 2);
}

#[test]
fn routed_event_accounts_only_for_retained_input_memory() {
    let total = SnapshotBatchBenchmark::total_conservative_bytes::<batching_lanes::SnapshotLanes, batching_lanes::InputLanes, _>([
        TestEvent::Positive(7, 10, 1, 1).into(),
    ]);

    // 42 retained event + 32 retained identity. Checkpoints reserve their exact
    // materialized size when they are committed by the worker.
    assert_eq!(74, total);
}
