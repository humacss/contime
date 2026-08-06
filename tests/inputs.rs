//! Verifies that events and plain markers share one generated input lane and one apply path.

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;

use contime::{ApplyInner, ApplyWrapper, Contime, ContimeError, Input, InputBatch, InputRoute, Marker, TestEvent, TestSnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuppressInput {
    id: u128,
    time: i64,
    event_id: u128,
    snapshot_ids: Vec<u128>,
}

impl Input for SuppressInput {
    type Time = i64;

    fn id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl Marker for SuppressInput {}

contime::lanes! {
    mod input_lanes;
    context SuppressClaimedEvents;
    snapshots [TestSnapshot];
    markers [SuppressInput];
    routes [
        TestEvent => [TestSnapshot],
    ];
}

impl InputRoute for SuppressInput {
    fn snapshot_ids(&self) -> Vec<u128> {
        self.snapshot_ids.clone()
    }
}

#[derive(Clone)]
struct SuppressClaimedEvents {
    applied_batches: Arc<AtomicUsize>,
}

#[derive(Clone, Default)]
struct CountPlainMarkers;

#[derive(Clone)]
struct RecordSnapshotTime {
    observed_time: Arc<AtomicI64>,
}

impl ApplyWrapper<input_lanes::SnapshotLanes> for CountPlainMarkers {
    fn apply_input_batch_wrapper(
        &mut self,
        snapshot: &mut input_lanes::SnapshotLanes,
        batch: InputBatch<'_, input_lanes::InputLanes>,
        apply_inner: ApplyInner<input_lanes::SnapshotLanes>,
    ) {
        let marker_count = batch.inputs.iter().filter(|input| matches!(input, input_lanes::InputLanes::SuppressInput(_))).count();
        apply_inner.apply_input_batch(snapshot, batch);

        let mut concrete: TestSnapshot = snapshot.clone().into();
        concrete.sum += marker_count as i32;
        *snapshot = concrete.into();
    }
}

impl ApplyWrapper<input_lanes::SnapshotLanes> for SuppressClaimedEvents {
    fn apply_input_batch_wrapper(
        &mut self,
        snapshot: &mut input_lanes::SnapshotLanes,
        batch: InputBatch<'_, input_lanes::InputLanes>,
        apply_inner: ApplyInner<input_lanes::SnapshotLanes>,
    ) {
        self.applied_batches.fetch_add(1, Ordering::Relaxed);
        let suppressed_ids = batch
            .inputs
            .iter()
            .filter_map(|input| match input {
                input_lanes::InputLanes::SuppressInput(marker) => Some(marker.event_id),
                input_lanes::InputLanes::TestEvent(_) => None,
            })
            .collect::<Vec<_>>();
        let inputs = batch
            .inputs
            .iter()
            .copied()
            .filter(|input| !matches!(input, input_lanes::InputLanes::TestEvent(event) if suppressed_ids.contains(&event.id())))
            .collect::<Vec<_>>();

        apply_inner.apply_input_batch(snapshot, InputBatch { snapshot_id: batch.snapshot_id, time: batch.time, inputs: &inputs });
    }
}

impl ApplyWrapper<input_lanes::SnapshotLanes> for RecordSnapshotTime {
    fn apply_input_batch_wrapper(
        &mut self,
        snapshot: &mut input_lanes::SnapshotLanes,
        batch: InputBatch<'_, input_lanes::InputLanes>,
        apply_inner: ApplyInner<input_lanes::SnapshotLanes>,
    ) {
        apply_inner.apply_input_batch(snapshot, batch);
        let concrete: TestSnapshot = snapshot.clone().into();
        self.observed_time.store(concrete.time, Ordering::Relaxed);
    }
}

#[test]
fn one_apply_call_batches_events_and_markers_through_the_same_input_lane() {
    let applied_batches = Arc::new(AtomicUsize::new(0));
    let contime =
        input_lanes::Contime::new_with_apply_context(1, 100_000, SuppressClaimedEvents { applied_batches: Arc::clone(&applied_batches) });

    contime
        .apply([
            SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] }.into(),
            TestEvent::Positive(1, 10, 100, 5).into(),
        ])
        .unwrap();

    let snapshot: TestSnapshot = contime.query_at(10, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 0, "the marker did not resolve the event submitted in the same input batch");
    assert_eq!(applied_batches.load(Ordering::Relaxed), 1, "one same-time input batch invoked the wrapper more than once");
    assert_eq!(contime.inspect_inputs(..).unwrap().len(), 2, "unified inspection did not retain both temporal inputs");
}

#[test]
fn duplicate_input_does_not_replay_the_history_again() {
    let applied_batches = Arc::new(AtomicUsize::new(0));
    let contime =
        input_lanes::Contime::new_with_apply_context(1, 100_000, SuppressClaimedEvents { applied_batches: Arc::clone(&applied_batches) });
    let marker = SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] };

    contime.apply([marker.clone().into()]).unwrap();
    let batches_after_first_apply = applied_batches.load(Ordering::Relaxed);
    contime.apply([marker.into()]).unwrap();

    assert_eq!(
        applied_batches.load(Ordering::Relaxed),
        batches_after_first_apply,
        "an identical input replayed a history that already contained it"
    );
}

#[test]
fn changed_input_with_the_same_identity_replays_and_replaces_the_inspected_value() {
    let contime =
        input_lanes::Contime::new_with_apply_context(1, 100_000, SuppressClaimedEvents { applied_batches: Arc::new(AtomicUsize::new(0)) });
    contime.apply([TestEvent::Positive(1, 10, 100, 5).into(), TestEvent::Positive(1, 10, 200, 7).into()]).unwrap();
    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] }.into()]).unwrap();

    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 200, snapshot_ids: vec![1] }.into()]).unwrap();

    let snapshot: TestSnapshot = contime.query_at(10, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 5, "replacing a marker input did not replay the batch with its changed value");
    let inputs = contime.inspect_inputs(..).unwrap();
    let inspected_marker = inputs
        .iter()
        .find_map(|entry| match &entry.input {
            input_lanes::InputLanes::SuppressInput(marker) => Some(marker),
            input_lanes::InputLanes::TestEvent(_) => None,
        })
        .expect("the retained marker input should remain inspectable");
    assert_eq!(inspected_marker.event_id, 200, "inspection retained the marker value that history replaced");
}

#[test]
fn event_and_marker_with_the_same_identity_replace_each_other() {
    type DefaultInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes>;
    let contime = DefaultInputContime::new(1, 100_000);
    contime.apply([TestEvent::Positive(1, 10, 100, 5).into()]).unwrap();

    contime.apply([SuppressInput { id: 100, time: 10, event_id: 100, snapshot_ids: vec![1] }.into()]).unwrap();

    let snapshot = contime.query_at(10, &[1]).unwrap().pop().flatten();
    assert!(snapshot.is_none(), "replacing the only event with a marker left its snapshot materialized");
    let inputs = contime.inspect_inputs(..).unwrap();
    assert_eq!(inputs.len(), 1, "an event and marker with one canonical identity were retained as separate inputs");
    assert!(
        matches!(inputs[0].input, input_lanes::InputLanes::SuppressInput(_)),
        "the replacement marker was not retained under the shared input identity"
    );
}

#[test]
fn plain_marker_creates_pending_history_without_materializing_a_snapshot() {
    let contime =
        input_lanes::Contime::new_with_apply_context(1, 100_000, SuppressClaimedEvents { applied_batches: Arc::new(AtomicUsize::new(0)) });

    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![7] }.into()]).unwrap();

    let snapshot = contime.query_at(10, &[7]).unwrap().pop().flatten();
    assert!(snapshot.is_none(), "a marker-only history materialized a snapshot without an event");
    assert_eq!(contime.inspect_inputs(..).unwrap().len(), 1, "the pending marker was not retained for later replay");
}

#[test]
fn later_event_materializes_a_pending_history_and_replays_its_markers() {
    let contime =
        input_lanes::Contime::new_with_apply_context(1, 100_000, SuppressClaimedEvents { applied_batches: Arc::new(AtomicUsize::new(0)) });
    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![7] }.into()]).unwrap();

    contime.apply([TestEvent::Positive(7, 10, 100, 5).into()]).unwrap();

    let snapshot: TestSnapshot = contime.query_at(10, &[7]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 0, "materialization did not replay the marker retained by the pending history");
}

#[test]
fn marker_only_batch_does_not_invoke_a_snapshot_apply_wrapper() {
    type RecordingInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes, RecordSnapshotTime>;
    let observed_time = Arc::new(AtomicI64::new(i64::MIN));
    let contime =
        RecordingInputContime::new_with_apply_context(1, 100_000, RecordSnapshotTime { observed_time: Arc::clone(&observed_time) });

    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![7] }.into()]).unwrap();

    assert_eq!(
        observed_time.load(Ordering::Relaxed),
        i64::MIN,
        "a marker-only pending history invoked an apply wrapper without a snapshot"
    );
}

#[test]
fn default_apply_wrapper_ignores_plain_markers() {
    type DefaultInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes>;
    let contime = DefaultInputContime::new(1, 100_000);

    contime
        .apply([
            TestEvent::Positive(1, 10, 100, 5).into(),
            SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] }.into(),
        ])
        .unwrap();

    let snapshot: TestSnapshot = contime.query_at(10, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 5, "the marker initialized snapshot state or was applied as an event");
}

#[test]
fn input_inspection_returns_one_global_input_with_every_route() {
    type DefaultInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes>;
    let contime = DefaultInputContime::new(2, 100_000);

    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![7, 3] }.into()]).unwrap();

    let entries = contime.inspect_inputs(..).unwrap();
    assert_eq!(entries.len(), 1, "one globally identified input was exposed once per route");
    assert_eq!(entries[0].routed_snapshot_ids, vec![3, 7], "input inspection lost or misordered routed snapshot IDs");
}

#[test]
fn all_inputs_use_the_same_history_horizon() {
    type DefaultInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes>;
    let contime = DefaultInputContime::with_history_horizon(1, 100_000, 50);
    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] }.into()]).unwrap();

    contime.advance_to(70).unwrap();

    assert!(contime.inspect_inputs(..).unwrap().is_empty(), "advancing the history horizon did not prune an old marker input");
    let error = contime.apply([SuppressInput { id: 2_000, time: 19, event_id: 200, snapshot_ids: vec![1] }.into()]).unwrap_err();
    assert!(
        matches!(error, ContimeError::InputBeforeHistoryHorizon { input_time: 19, earliest_time: 20 }),
        "an input before the retained history horizon returned the wrong error: {error:?}"
    );
}

#[test]
fn pruning_a_marker_only_history_keeps_it_pending() {
    type CountingInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes, CountPlainMarkers>;
    let contime = CountingInputContime::with_history_horizon_and_apply_context(1, 100_000, 50, CountPlainMarkers);
    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] }.into()]).unwrap();

    contime.advance_to(70).unwrap();

    let snapshot = contime.query_at(70, &[1]).unwrap().pop().flatten();
    assert!(snapshot.is_none(), "pruning a marker-only history materialized a snapshot");
}

#[test]
fn pruning_a_materialized_history_retains_a_complete_replay_anchor() {
    type CountingInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes, CountPlainMarkers>;
    let contime = CountingInputContime::with_history_horizon_and_apply_context(1, 100_000, 50, CountPlainMarkers);
    contime
        .apply([
            TestEvent::Positive(1, 10, 100, 5).into(),
            SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] }.into(),
        ])
        .unwrap();

    contime.advance_to(70).unwrap();

    let snapshot: TestSnapshot = contime.query_at(70, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 6, "pruning discarded event or marker effects from the retained replay anchor");
}
