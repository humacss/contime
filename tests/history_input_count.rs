//! Verifies the cumulative raw-input frontier supplied during snapshot replay.

use std::sync::{Arc, Mutex};

use contime::{
    ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, Event, Input, InputBatch, InputRoute, Marker, Snapshot, SnapshotEvent,
    SnapshotHistory,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CountAwareSnapshot {
    id: u128,
    time: i64,
    history_input_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CountAwareEvent {
    id: u128,
    snapshot_id: u128,
    time: i64,
}

impl Input for CountAwareEvent {
    type Time = i64;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        size_of::<Self>() as u64
    }
}

impl Event for CountAwareEvent {}

impl SnapshotEvent<CountAwareSnapshot> for CountAwareEvent {
    fn snapshot_id(&self) -> u128 {
        self.snapshot_id
    }

    fn set_snapshot_identity(&self, snapshot: &mut CountAwareSnapshot) {
        snapshot.id = self.snapshot_id;
    }
}

impl Snapshot for CountAwareSnapshot {
    type Time = i64;
    type Input = CountAwareEvent;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time
    }

    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        size_of::<Self>() as u64
    }
}

impl ApplyEvents<CountAwareEvent> for CountAwareSnapshot {
    fn apply_events(&mut self, batch: ApplyBatch<'_, CountAwareEvent>) {
        self.history_input_count = batch.history_input_count;
    }
}

contime::lanes! {
    mod count_lanes;
    snapshots [CountAwareSnapshot];
    routes [
        CountAwareEvent => [CountAwareSnapshot],
    ];
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SuppressEvent {
    id: u128,
    event_id: u128,
    snapshot_id: u128,
    time: i64,
}

impl Input for SuppressEvent {
    type Time = i64;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        size_of::<Self>() as u64
    }
}

impl Marker for SuppressEvent {}

impl InputRoute for SuppressEvent {
    fn visit_snapshot_ids<F>(&self, visit: &mut F)
    where
        F: FnMut(u128),
    {
        visit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct CountTrace {
    observations: Arc<Mutex<Vec<(u64, usize)>>>,
}

impl ApplyWrapper<marker_lanes::SnapshotLanes> for CountTrace {
    fn apply_input_batch_wrapper(
        &mut self,
        batch: InputBatch<'_, marker_lanes::InputLanes>,
        apply_inner: &mut ApplyInner<'_, marker_lanes::SnapshotLanes>,
    ) {
        let suppressed_ids = batch
            .inputs
            .iter()
            .filter_map(|input| match input {
                marker_lanes::InputLanes::SuppressEvent(marker) => Some(marker.event_id),
                marker_lanes::InputLanes::CountAwareEvent(_) => None,
            })
            .collect::<Vec<_>>();
        let effective = batch
            .inputs
            .iter()
            .copied()
            .filter(|input| {
                !matches!(
                    input,
                    marker_lanes::InputLanes::CountAwareEvent(event)
                        if suppressed_ids.contains(&event.id)
                )
            })
            .collect::<Vec<_>>();

        apply_inner.apply_input_batch(InputBatch { snapshot_id: batch.snapshot_id, time: batch.time, inputs: &effective });
        self.observations.lock().expect("history count trace lock should remain available").push((
            apply_inner.history_input_count(),
            effective.iter().filter(|input| matches!(input, marker_lanes::InputLanes::CountAwareEvent(_))).count(),
        ));
    }
}

contime::lanes! {
    mod marker_lanes;
    context CountTrace;
    snapshots [CountAwareSnapshot];
    markers [SuppressEvent];
    routes [
        CountAwareEvent => [CountAwareSnapshot],
    ];
}

#[derive(Clone, Default)]
struct PartitionTrace {
    observed_counts: Arc<Mutex<Vec<u64>>>,
}

impl ApplyWrapper<partition_lanes::SnapshotLanes> for PartitionTrace {
    fn apply_input_batch_wrapper(
        &mut self,
        batch: InputBatch<'_, partition_lanes::InputLanes>,
        apply_inner: &mut ApplyInner<'_, partition_lanes::SnapshotLanes>,
    ) {
        for input in batch.inputs {
            apply_inner.apply_input_batch(InputBatch { snapshot_id: batch.snapshot_id, time: batch.time, inputs: &[*input] });
            self.observed_counts
                .lock()
                .expect("partition count trace lock should remain available")
                .push(apply_inner.history_input_count());
        }
    }
}

contime::lanes! {
    mod partition_lanes;
    context PartitionTrace;
    snapshots [CountAwareSnapshot];
    routes [
        CountAwareEvent => [CountAwareSnapshot],
    ];
}

#[test]
fn concrete_apply_receives_the_cumulative_raw_history_input_count() {
    let contime = count_lanes::Contime::new(1, 1_000_000);

    contime
        .apply([CountAwareEvent { id: 100, snapshot_id: 7, time: 10 }.into(), CountAwareEvent { id: 200, snapshot_id: 7, time: 20 }.into()])
        .expect("the count-aware history should apply");

    let snapshot = contime
        .query_at(20, &[7])
        .expect("the count-aware snapshot should be queryable")
        .pop()
        .flatten()
        .expect("the event history should materialize its snapshot");

    let count_lanes::SnapshotLanes::CountAwareSnapshot(snapshot) = snapshot;

    assert_eq!(2, snapshot.history_input_count, "snapshot application did not receive the cumulative raw history input count");
}

#[test]
fn marker_only_effective_batch_still_advances_the_raw_history_input_count() {
    let trace = CountTrace::default();
    let contime = marker_lanes::Contime::new_with_apply_context(1, 1_000_000, trace.clone());

    contime.apply([CountAwareEvent { id: 100, snapshot_id: 7, time: 10 }.into()]).expect("the event should initially apply");
    trace.observations.lock().expect("history count trace lock should remain available").clear();

    contime
        .apply([SuppressEvent { id: 1_000, event_id: 100, snapshot_id: 7, time: 10 }.into()])
        .expect("the late marker should replay and suppress the event");

    assert_eq!(
        vec![(2, 0)],
        *trace.observations.lock().expect("history count trace lock should remain available"),
        "the marker or filtered event was omitted from the raw history input count"
    );
}

#[test]
fn duplicate_input_does_not_advance_the_history_input_count() {
    let contime = count_lanes::Contime::new(1, 1_000_000);
    let event = CountAwareEvent { id: 100, snapshot_id: 7, time: 10 };

    contime.apply([event.clone().into()]).expect("the first event should apply");
    contime.apply([event.into()]).expect("the duplicate event should be an idempotent no-op");

    let snapshot = contime.query_at(10, &[7]).unwrap().pop().flatten().unwrap();
    let count_lanes::SnapshotLanes::CountAwareSnapshot(snapshot) = snapshot;
    assert_eq!(1, snapshot.history_input_count, "a duplicate identity advanced the raw history input count");
}

#[test]
fn same_time_incremental_insertion_replays_to_the_complete_bucket_count() {
    let contime = count_lanes::Contime::new(1, 1_000_000);

    contime.apply([CountAwareEvent { id: 100, snapshot_id: 7, time: 10 }.into()]).expect("the first same-time input should apply");
    contime
        .apply([CountAwareEvent { id: 200, snapshot_id: 7, time: 10 }.into()])
        .expect("the second same-time input should replay the bucket");

    let snapshot = contime.query_at(10, &[7]).unwrap().pop().flatten().unwrap();
    let count_lanes::SnapshotLanes::CountAwareSnapshot(snapshot) = snapshot;
    assert_eq!(2, snapshot.history_input_count, "same-time replay did not expose the complete raw bucket frontier");
}

#[test]
fn out_of_order_replay_recalculates_later_history_input_counts() {
    let contime = count_lanes::Contime::new(1, 1_000_000);

    contime
        .apply([CountAwareEvent { id: 100, snapshot_id: 7, time: 10 }.into(), CountAwareEvent { id: 300, snapshot_id: 7, time: 30 }.into()])
        .expect("the initial history should apply");
    contime.apply([CountAwareEvent { id: 200, snapshot_id: 7, time: 20 }.into()]).expect("the late input should replay the later history");

    let snapshot = contime.query_at(30, &[7]).unwrap().pop().flatten().unwrap();
    let count_lanes::SnapshotLanes::CountAwareSnapshot(snapshot) = snapshot;
    assert_eq!(3, snapshot.history_input_count, "late replay did not advance the later raw history frontier");
}

#[test]
fn checkpoint_restores_the_history_input_count_before_replay() {
    let (mut history, _bytes) = SnapshotHistory::<count_lanes::SnapshotLanes>::new_with_snapshot_id(7, 0, 0);
    history.checkpoint_interval = 1;
    let mut context = ();
    history.apply_input_batch(vec![CountAwareEvent { id: 100, snapshot_id: 7, time: 10 }.into()], &mut context);
    history.apply_input_batch(vec![CountAwareEvent { id: 300, snapshot_id: 7, time: 30 }.into()], &mut context);

    history.apply_input_batch(vec![CountAwareEvent { id: 200, snapshot_id: 7, time: 20 }.into()], &mut context);

    let snapshot = history.snapshot_at(30);
    let count_lanes::SnapshotLanes::CountAwareSnapshot(snapshot) = snapshot;
    assert_eq!(3, snapshot.history_input_count, "replay did not continue from the count retained by its checkpoint");
    assert!(
        history.checkpoints.iter().any(|(key, _snapshot, count)| key.time == 10 && *count == 1),
        "the checkpoint did not retain its cumulative raw history input count"
    );
}

#[test]
fn wrapper_partitions_share_one_raw_history_input_count() {
    let trace = PartitionTrace::default();
    let contime = partition_lanes::Contime::new_with_apply_context(1, 1_000_000, trace.clone());

    contime
        .apply([CountAwareEvent { id: 100, snapshot_id: 7, time: 10 }.into(), CountAwareEvent { id: 200, snapshot_id: 7, time: 10 }.into()])
        .expect("the partitioned raw input bucket should apply");

    assert_eq!(
        vec![2, 2],
        *trace.observed_counts.lock().expect("partition count trace lock should remain available"),
        "one raw input bucket was counted separately for each effective partition"
    );
}

#[test]
fn advancing_the_horizon_does_not_reset_the_history_input_count() {
    let (mut history, _bytes) = SnapshotHistory::<count_lanes::SnapshotLanes>::new_with_snapshot_id(7, 0, 10);
    let mut context = ();
    history.apply_input_batch(vec![CountAwareEvent { id: 100, snapshot_id: 7, time: 10 }.into()], &mut context);
    history.apply_input_batch(vec![CountAwareEvent { id: 200, snapshot_id: 7, time: 20 }.into()], &mut context);

    history.advance(25);
    history.apply_input_batch(vec![CountAwareEvent { id: 300, snapshot_id: 7, time: 30 }.into()], &mut context);

    let snapshot = history.snapshot_at(30);
    let count_lanes::SnapshotLanes::CountAwareSnapshot(snapshot) = snapshot;
    assert_eq!(3, snapshot.history_input_count, "horizon compaction reset the cumulative raw history input frontier");
}
