//! Verifies that events and plain markers share one generated input lane and one apply path.

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;

use contime::{
    ApplyInner, ApplyWrapper, Contime, EventRejection, EventRejectionReason, Input, InputBatch, InputRoute, Marker, TestEvent, TestSnapshot,
};

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
    fn visit_snapshot_ids<F>(&self, visit: &mut F)
    where
        F: FnMut(u128),
    {
        for &snapshot_id in &self.snapshot_ids {
            visit(snapshot_id);
        }
    }
}

#[test]
fn marker_route_visitor_preserves_dynamic_order_and_empty_routes() {
    let routed = SuppressInput { id: 1, time: 10, event_id: 2, snapshot_ids: vec![7, 3] };
    let mut visited = Vec::new();
    <SuppressInput as InputRoute>::visit_snapshot_ids(&routed, &mut |snapshot_id| visited.push(snapshot_id));
    assert_eq!(visited, vec![7, 3]);

    let unrouted = SuppressInput { id: 3, time: 10, event_id: 4, snapshot_ids: Vec::new() };
    <SuppressInput as InputRoute>::visit_snapshot_ids(&unrouted, &mut |snapshot_id| visited.push(snapshot_id));
    assert_eq!(visited, vec![7, 3]);
}

#[derive(Clone)]
struct SuppressClaimedEvents {
    applied_batches: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct RecordSnapshotTime {
    observed_time: Arc<AtomicI64>,
}

impl ApplyWrapper<input_lanes::SnapshotLanes> for SuppressClaimedEvents {
    fn apply_input_batch_wrapper(
        &mut self,
        batch: InputBatch<'_, input_lanes::InputLanes>,
        apply_inner: &mut ApplyInner<'_, input_lanes::SnapshotLanes>,
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

        apply_inner.apply_input_batch(InputBatch { snapshot_id: batch.snapshot_id, time: batch.time, inputs: &inputs });
    }
}

impl ApplyWrapper<input_lanes::SnapshotLanes> for RecordSnapshotTime {
    fn apply_input_batch_wrapper(
        &mut self,
        batch: InputBatch<'_, input_lanes::InputLanes>,
        apply_inner: &mut ApplyInner<'_, input_lanes::SnapshotLanes>,
    ) {
        apply_inner.apply_input_batch(batch);
        let concrete: TestSnapshot = apply_inner.snapshot().clone().into();
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

    assert_eq!(applied_batches.load(Ordering::Relaxed), 1, "one same-time input batch invoked the wrapper more than once");
    let snapshot: TestSnapshot = contime.query_at(10, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 0, "the marker did not suppress the event submitted in the same input batch");
}

#[test]
fn marker_added_after_an_event_replays_from_before_the_shared_timestamp() {
    let applied_batches = Arc::new(AtomicUsize::new(0));
    let contime =
        input_lanes::Contime::new_with_apply_context(1, 100_000, SuppressClaimedEvents { applied_batches: Arc::clone(&applied_batches) });
    contime.apply([TestEvent::Positive(1, 10, 100, 5).into()]).unwrap();

    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] }.into()]).unwrap();

    let snapshot: TestSnapshot = contime.query_at(10, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 0, "a marker added after its target event reused a checkpoint containing the suppressed event");
    assert!(applied_batches.load(Ordering::Relaxed) >= 2, "the late marker did not replay its target timestamp");
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
fn changed_payload_with_the_same_identity_is_a_noop() {
    let contime =
        input_lanes::Contime::new_with_apply_context(1, 100_000, SuppressClaimedEvents { applied_batches: Arc::new(AtomicUsize::new(0)) });
    contime.apply([TestEvent::Positive(1, 10, 100, 5).into(), TestEvent::Positive(1, 10, 200, 7).into()]).unwrap();
    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] }.into()]).unwrap();

    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 200, snapshot_ids: vec![1] }.into()]).unwrap();

    let snapshot: TestSnapshot = contime.query_at(10, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 7, "a duplicate identity replaced the first canonical marker payload");
}

#[test]
fn event_and_marker_with_the_same_identity_keep_the_first_input() {
    type DefaultInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes>;
    let contime = DefaultInputContime::new(1, 100_000);
    contime.apply([TestEvent::Positive(1, 10, 100, 5).into()]).unwrap();

    contime.apply([SuppressInput { id: 100, time: 10, event_id: 100, snapshot_ids: vec![1] }.into()]).unwrap();

    let snapshot: TestSnapshot = contime.query_at(10, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 5, "a marker with an occupied identity replaced the canonical event");
}

#[test]
fn plain_marker_creates_pending_history_without_materializing_a_snapshot() {
    let contime =
        input_lanes::Contime::new_with_apply_context(1, 100_000, SuppressClaimedEvents { applied_batches: Arc::new(AtomicUsize::new(0)) });

    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![7] }.into()]).unwrap();

    let snapshot = contime.query_at(10, &[7]).unwrap().pop().flatten();
    assert!(snapshot.is_none(), "a marker-only history materialized a snapshot without an event");
}

#[test]
fn later_suppressed_event_materializes_identity_without_applying_event_state() {
    let contime =
        input_lanes::Contime::new_with_apply_context(1, 100_000, SuppressClaimedEvents { applied_batches: Arc::new(AtomicUsize::new(0)) });
    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![7] }.into()]).unwrap();

    contime.apply([TestEvent::Positive(7, 10, 100, 5).into()]).unwrap();

    let snapshot: TestSnapshot = contime.query_at(10, &[7]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 0, "a suppressed event applied state while materializing the pending history identity");
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
fn forwarded_marker_reaches_lane_application_after_snapshot_materialization() {
    type RecordingInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes, RecordSnapshotTime>;
    let observed_time = Arc::new(AtomicI64::new(i64::MIN));
    let contime =
        RecordingInputContime::new_with_apply_context(1, 100_000, RecordSnapshotTime { observed_time: Arc::clone(&observed_time) });
    contime.apply([TestEvent::Positive(7, 5, 100, 3).into()]).unwrap();

    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![7] }.into()]).unwrap();

    assert_eq!(
        observed_time.load(Ordering::Relaxed),
        10,
        "a forwarded marker batch did not reach lane application for a materialized snapshot",
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
fn stale_inputs_are_reported_while_valid_batch_inputs_apply() {
    type DefaultInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes>;
    let contime = DefaultInputContime::with_history_horizon(1, 100_000, 50);
    contime.advance_to(70).unwrap();

    let rejections = contime
        .apply([TestEvent::Positive(1, 19, 200, 5).into(), TestEvent::Positive(1, 25, 300, 7).into()])
        .expect("a stale input should not reject valid inputs in the same batch");

    assert_eq!(rejections, vec![EventRejection::new(200, EventRejectionReason::BeforeHistoryHorizon)]);
    let snapshot: TestSnapshot =
        contime.query_at(70, &[1]).unwrap().pop().flatten().expect("the valid input should materialize the snapshot").into();
    assert_eq!(snapshot.sum, 7);
}

#[test]
fn unrouted_inputs_do_not_consume_the_memory_budget() {
    type DefaultInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes>;
    let contime = DefaultInputContime::new(1, 1);

    contime
        .apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: Vec::new() }.into()])
        .expect("an unrouted input should be discarded before memory accounting");
}

#[test]
fn unrouted_inputs_are_discarded_before_history_horizon_validation() {
    type DefaultInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes>;
    let contime = DefaultInputContime::with_history_horizon(1, 100_000, 50);
    contime.advance_to(100).unwrap();

    contime
        .apply([SuppressInput { id: 1_000, time: 49, event_id: 100, snapshot_ids: Vec::new() }.into()])
        .expect("an unrouted input should be discarded before horizon validation");
}

#[test]
fn pruning_a_marker_only_history_keeps_it_pending() {
    type SuppressingInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes, SuppressClaimedEvents>;
    let contime = SuppressingInputContime::with_history_horizon_and_apply_context(
        1,
        100_000,
        50,
        SuppressClaimedEvents { applied_batches: Arc::new(AtomicUsize::new(0)) },
    );
    contime.apply([SuppressInput { id: 1_000, time: 10, event_id: 100, snapshot_ids: vec![1] }.into()]).unwrap();

    contime.advance_to(70).unwrap();

    let snapshot = contime.query_at(70, &[1]).unwrap().pop().flatten();
    assert!(snapshot.is_none(), "pruning a marker-only history materialized a snapshot");
}

#[test]
fn pruning_a_materialized_history_retains_a_complete_replay_anchor() {
    type SuppressingInputContime = Contime<input_lanes::SnapshotLanes, input_lanes::InputLanes, SuppressClaimedEvents>;
    let contime = SuppressingInputContime::with_history_horizon_and_apply_context(
        1,
        100_000,
        50,
        SuppressClaimedEvents { applied_batches: Arc::new(AtomicUsize::new(0)) },
    );
    contime
        .apply([
            TestEvent::Positive(1, 10, 100, 5).into(),
            TestEvent::Positive(1, 10, 200, 7).into(),
            SuppressInput { id: 1_000, time: 10, event_id: 200, snapshot_ids: vec![1] }.into(),
        ])
        .unwrap();

    contime.advance_to(70).unwrap();

    let snapshot: TestSnapshot = contime.query_at(70, &[1]).unwrap().pop().flatten().unwrap().into();
    assert_eq!(snapshot.sum, 5, "pruning discarded effective-event filtering from the retained replay anchor");
}
