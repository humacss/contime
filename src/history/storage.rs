use std::collections::VecDeque;

use crate::{ContimeKey, ContimeTime, InputLanes, Snapshot, SnapshotLanes};

use super::checkpoints::{get_checkpoint_at, get_checkpoint_at_with_context, get_checkpoint_before_with_context};
use super::HistoryInputs;

type SnapshotId = u128;

/// Per-snapshot history store used internally by `Contime`.
///
/// Most users should interact with [`crate::Contime`] instead. This type is useful when you
/// want direct control over one snapshot timeline, for example in benchmarks or focused tests.
#[derive(Debug, Clone)]
pub struct LocalSnapshotHistory<S>
where
    S: Snapshot,
{
    /// Snapshot id owned by this history.
    pub snapshot_id: SnapshotId,
    /// In-memory snapshot-lane variant established by the first event.
    pub(super) snapshot_lane_index: Option<usize>,
    /// Materialized checkpoints sorted by event time and id.
    pub checkpoints: VecDeque<(ContimeKey<S::Time>, S, u64)>,
    /// Checkpoint containing the accumulated state of inputs pruned before the horizon.
    pub(super) replay_anchor_key: Option<ContimeKey<S::Time>>,
    /// Routed temporal inputs exposed through representation-neutral ordered iteration.
    pub inputs: HistoryInputs<S::Time, S::Input>,
    /// Interval between generated checkpoints during replay.
    pub checkpoint_interval: usize,

    current_time: S::Time,
    lower_time_horizon_delta: S::Time,
}

const CHECKPOINT_INTERVAL: usize = 100;

impl<S> LocalSnapshotHistory<S>
where
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
{
    /// Creates a history for one snapshot and returns it with its initial memory cost.
    pub fn new(snapshot: S, current_time: S::Time, lower_time_horizon_delta: S::Time) -> (Self, i64) {
        let snapshot_id = snapshot.id();
        let snapshot_lane_index = snapshot.lane_index();
        let snapshot_size = snapshot.conservative_size() as i64 + size_of::<u64>() as i64;
        let checkpoint_key = ContimeKey { time: snapshot.time(), id: u128::MAX };
        let mut checkpoints = VecDeque::new();
        checkpoints.push_back((checkpoint_key, snapshot, 0));
        (
            Self {
                current_time,
                lower_time_horizon_delta,
                snapshot_id,
                snapshot_lane_index: Some(snapshot_lane_index),
                checkpoints,
                replay_anchor_key: None,
                inputs: HistoryInputs::new(),
                checkpoint_interval: CHECKPOINT_INTERVAL,
            },
            snapshot_size,
        )
    }

    /// Creates a pending history for one explicitly routed snapshot id.
    pub fn new_with_snapshot_id(snapshot_id: u128, current_time: S::Time, lower_time_horizon_delta: S::Time) -> (Self, i64) {
        (
            Self {
                current_time,
                lower_time_horizon_delta,
                snapshot_id,
                snapshot_lane_index: None,
                checkpoints: VecDeque::new(),
                replay_anchor_key: None,
                inputs: HistoryInputs::new(),
                checkpoint_interval: CHECKPOINT_INTERVAL,
            },
            0,
        )
    }

    /// Advances the internal current time to `time` and prunes history outside the configured horizon.
    pub fn advance(&mut self, time: S::Time) -> i64 {
        let mut context = ();
        self.advance_with_context(time, &mut context)
    }

    pub(crate) fn advance_with_context<C>(&mut self, time: S::Time, context: &mut C) -> i64
    where
        C: crate::ApplyWrapper<S>,
    {
        if time <= self.current_time {
            return 0;
        }

        self.current_time = time;
        let drop_time = self.current_time.clone().saturating_sub(self.lower_time_horizon_delta.clone());
        let drop_key = ContimeKey { time: drop_time.clone(), id: u128::MIN };

        let mut bytes_delta: i64 = 0;

        let replay_anchor = get_checkpoint_before_with_context(self, drop_time.clone(), context);

        let first_kept_checkpoint = self.checkpoints.partition_point(|(key, _checkpoint, _history_input_count)| key < &drop_key);
        for (_key, checkpoint, _history_input_count) in self.checkpoints.drain(..first_kept_checkpoint) {
            bytes_delta -= checkpoint.conservative_size() as i64 + size_of::<u64>() as i64;
        }

        for (_key, checkpoint, _history_input_count) in &mut self.checkpoints {
            let previous_size = checkpoint.conservative_size();
            checkpoint.compact_before(drop_time.clone());
            bytes_delta += checkpoint.conservative_size() as i64 - previous_size as i64;
        }

        if let Some((key, mut checkpoint, history_input_count)) = replay_anchor {
            checkpoint.compact_before(drop_time);
            if self.checkpoints.front().is_none_or(|(existing_key, _checkpoint, _history_input_count)| existing_key > &key) {
                bytes_delta += checkpoint.conservative_size() as i64 + size_of::<u64>() as i64;
                self.checkpoints.push_front((key.clone(), checkpoint, history_input_count));
            }
            self.replay_anchor_key = Some(key);
        }

        let input_count_before_prune = self.inputs.len();
        let pruned = self.inputs.prune_before(&drop_key);
        debug_assert_eq!(input_count_before_prune - self.inputs.len(), pruned.count());
        bytes_delta -= pruned.bytes() as i64;

        bytes_delta
    }

    pub(crate) fn earliest_retained_time(&self) -> S::Time {
        self.current_time.clone().saturating_sub(self.lower_time_horizon_delta.clone())
    }

    pub(crate) fn conservative_replay_reservation(&self) -> u64 {
        self.checkpoints
            .iter()
            .map(|(_key, checkpoint, _history_input_count)| checkpoint.conservative_size().saturating_add(size_of::<u64>() as u64))
            .max()
            .unwrap_or(0)
    }

    /// Reconstructs the snapshot state at `time`.
    ///
    /// Events at exactly `time` are included in the returned snapshot.
    pub fn snapshot_at(&self, time: S::Time) -> S {
        get_checkpoint_at(self, time).expect("snapshot history is pending because it has no event")
    }

    /// Reconstructs the snapshot state at `time`.
    ///
    /// Events at exactly `time` are included in the returned snapshot.
    pub fn snapshot_only_at(&self, time: S::Time) -> S {
        get_checkpoint_at(self, time).expect("snapshot history is pending because it has no event")
    }

    /// Reconstructs the snapshot state at `time` using the provided apply wrapper.
    ///
    /// Events at exactly `time` are included in the returned snapshot.
    pub(crate) fn snapshot_only_at_with_context<C>(&self, time: S::Time, context: &mut C) -> Option<S>
    where
        C: crate::ApplyWrapper<S>,
    {
        get_checkpoint_at_with_context(self, time, context)
    }
}

/// Concrete per-snapshot history type used by the crate.
pub type SnapshotHistory<S> = LocalSnapshotHistory<S>;

#[cfg(test)]
mod tests {
    #![allow(unused_must_use)]

    use super::*;

    use crate::{
        ApplyBatch, Event, EventRejection, EventRejectionReason, Input, InputBatch, InputLanes, SnapshotEvent, TestEvent, TestSnapshot,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ContextEvent {
        id: u128,
        time: i64,
        snapshot_id: u128,
        value: i32,
    }

    impl Input for ContextEvent {
        type Time = i64;

        fn id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> i64 {
            self.time
        }

        fn conservative_size(&self) -> u64 {
            16 + 8 + 16 + 4
        }
    }

    impl Event for ContextEvent {}

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    struct ContextSnapshot {
        id: u128,
        time: i64,
        sum: i32,
    }

    impl Snapshot for ContextSnapshot {
        type Time = i64;
        type Input = ContextEvent;

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

    impl crate::SnapshotLanes for ContextSnapshot {
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

    impl SnapshotEvent<ContextSnapshot> for ContextEvent {
        fn snapshot_id(&self) -> u128 {
            self.snapshot_id
        }

        fn set_snapshot_identity(&self, snapshot: &mut ContextSnapshot) {
            snapshot.id = self.snapshot_id;
        }
    }

    impl crate::ApplyEvents<ContextEvent> for ContextSnapshot {
        fn apply_events(&mut self, batch: ApplyBatch<'_, ContextEvent>) {
            self.id = batch.snapshot_id;
            for event in batch.events.iter().copied() {
                self.sum += event.value;
            }
            self.set_time(batch.time);
        }
    }

    impl crate::ApplyWrapper<ContextSnapshot> for Vec<i32> {
        fn apply_input_batch_wrapper(
            &mut self,
            batch: InputBatch<'_, ContextEvent>,
            apply_inner: &mut crate::ApplyInner<'_, ContextSnapshot>,
        ) {
            apply_inner.apply_input_batch(batch);
            self.push(apply_inner.snapshot().sum);
        }
    }

    fn checkpoint_keys<S: Snapshot>(history: &SnapshotHistory<S>) -> Vec<ContimeKey<S::Time>> {
        history.checkpoints.iter().map(|(key, _checkpoint, _history_input_count)| key.clone()).collect()
    }

    fn checkpoint<'a, S: Snapshot>(history: &'a SnapshotHistory<S>, key: &ContimeKey<S::Time>) -> Option<&'a S> {
        history
            .checkpoints
            .iter()
            .find(|(checkpoint_key, _checkpoint, _history_input_count)| checkpoint_key == key)
            .map(|(_key, checkpoint, _history_input_count)| checkpoint)
    }

    fn first_checkpoint<S: Snapshot>(history: &SnapshotHistory<S>) -> Option<&S> {
        history.checkpoints.front().map(|(_key, checkpoint, _history_input_count)| checkpoint)
    }

    fn apply_one<S>(history: &mut SnapshotHistory<S>, event: S::Input)
    where
        S: crate::SnapshotLanes + 'static,
        S::Input: InputLanes<S>,
    {
        history.apply_input_batch(vec![event], &mut ());
    }

    #[test]
    fn duplicate_id_at_a_different_time_is_local_to_one_history() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut first, _) = SnapshotHistory::new(snapshot.clone(), 0, 50);
        let (mut second, _) = SnapshotHistory::new(snapshot, 0, 50);

        first.apply_input_batch(vec![TestEvent::Positive(1, 10, 7, 10)], &mut ());
        first.apply_input_batch(vec![TestEvent::Positive(1, 20, 7, 99)], &mut ());
        second.apply_input_batch(vec![TestEvent::Positive(1, 20, 7, 99)], &mut ());

        assert_eq!(first.snapshot_only_at(20).sum, 10);
        assert_eq!(second.snapshot_only_at(20).sum, 99);
    }

    #[test]
    fn pruning_forgets_identity_in_that_history() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 50);
        history.apply_input_batch(vec![TestEvent::Positive(1, 10, 7, 10)], &mut ());
        history.apply_input_batch(vec![TestEvent::Positive(1, 15, 7, 99)], &mut ());
        assert_eq!(history.snapshot_only_at(15).sum, 10);

        history.advance(70);
        history.apply_input_batch(vec![TestEvent::Positive(1, 30, 7, 20)], &mut ());

        assert_eq!(history.snapshot_only_at(70).sum, 30);
    }

    #[test]
    fn routed_apply_rejects_input_before_this_history_horizon() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 50);
        history.advance(100);

        let result = history.apply_routed_input_batch(vec![TestEvent::Positive(1, 49, 7, 10)], &mut ());

        assert_eq!(result.bytes_delta, 0);
        assert_eq!(result.rejections, vec![EventRejection::new(7, EventRejectionReason::BeforeHistoryHorizon)]);
    }

    #[test]
    fn in_order_apply_updates_current_end_checkpoint_every_time() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        history.checkpoint_interval = 100;

        apply_one(&mut history, TestEvent::Positive(1, 1, 1, 5));
        assert_eq!(checkpoint_keys(&history), vec![ContimeKey { time: 1, id: 1 }]);
        assert_eq!(first_checkpoint(&history).expect("checkpoint").sum, 5);

        apply_one(&mut history, TestEvent::Positive(1, 2, 2, 7));
        assert_eq!(checkpoint_keys(&history), vec![ContimeKey { time: 2, id: 2 }]);
        assert_eq!(first_checkpoint(&history).expect("checkpoint").sum, 12);
    }

    #[test]
    fn in_order_apply_preserves_cadence_anchor_and_moves_current_end_checkpoint() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        history.checkpoint_interval = 2;

        apply_one(&mut history, TestEvent::Positive(1, 1, 1, 5));
        apply_one(&mut history, TestEvent::Positive(1, 2, 2, 7));
        apply_one(&mut history, TestEvent::Positive(1, 3, 3, 11));

        assert_eq!(checkpoint_keys(&history), vec![ContimeKey { time: 2, id: 2 }, ContimeKey { time: 3, id: 3 }]);
        assert_eq!(checkpoint(&history, &ContimeKey { time: 2, id: 2 }).expect("anchor").sum, 12);
        assert_eq!(checkpoint(&history, &ContimeKey { time: 3, id: 3 }).expect("tip").sum, 23);
    }

    #[test]
    fn out_of_order_apply_overwrites_later_checkpoints_without_deleting_them() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        history.checkpoint_interval = 2;

        apply_one(&mut history, TestEvent::Positive(1, 10, 10, 10));
        apply_one(&mut history, TestEvent::Positive(1, 20, 20, 20));
        apply_one(&mut history, TestEvent::Positive(1, 30, 30, 30));
        assert_eq!(checkpoint_keys(&history), vec![ContimeKey { time: 20, id: 20 }, ContimeKey { time: 30, id: 30 }]);

        apply_one(&mut history, TestEvent::Positive(1, 15, 15, 15));

        assert_eq!(checkpoint_keys(&history), vec![ContimeKey { time: 15, id: 15 }, ContimeKey { time: 30, id: 30 }]);
        assert_eq!(checkpoint(&history, &ContimeKey { time: 15, id: 15 }).expect("checkpoint").sum, 25);
        assert_eq!(checkpoint(&history, &ContimeKey { time: 30, id: 30 }).expect("tip").sum, 75);
    }

    #[test]
    fn out_of_order_apply_replays_existing_future_bucket_from_corrected_state() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 10_000);
        history.checkpoint_interval = 100;

        apply_one(&mut history, TestEvent::Positive(1, 1, 1, 1));
        apply_one(&mut history, TestEvent::Positive(1, 1001, 1001, 1000));
        assert_eq!(history.snapshot_only_at(1100).sum, 1001);

        apply_one(&mut history, TestEvent::Positive(1, 876, 876, 10));

        let actual = history.snapshot_only_at(1100);
        assert_eq!(actual.sum, 1011);
        assert_eq!(actual.items, vec![1, 10, 1000]);
    }

    #[test]
    fn snapshot_at_includes_events_at_query_time() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);

        apply_one(&mut history, TestEvent::Positive(1, 10, 1, 5));

        let actual = history.snapshot_only_at(10);

        assert_eq!(actual.sum, 5);
        assert_eq!(actual.time, 10);
    }

    #[test]
    fn same_millisecond_events_replay_in_event_id_order() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);

        apply_one(&mut history, TestEvent::Positive(1, 10, 20, 2));
        apply_one(&mut history, TestEvent::Negative(1, 10, 10, 1));

        let actual = history.snapshot_only_at(10);

        assert_eq!(actual.items, vec![-1, 2]);
        assert_eq!(actual.sum, 1);
    }

    #[test]
    fn ordered_then_late_history_replays_across_both_stores() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1_000);
        apply_one(&mut history, TestEvent::Positive(1, 10, 10, 1));
        apply_one(&mut history, TestEvent::Positive(1, 30, 30, 3));

        apply_one(&mut history, TestEvent::Positive(1, 20, 20, 2));

        assert_eq!(history.inputs.storage_counts(), (2, 1));
        assert_eq!(history.snapshot_only_at(30).items, vec![1, 2, 3]);
    }

    #[test]
    fn same_time_late_id_replays_the_complete_bucket_in_id_order() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1_000);
        apply_one(&mut history, TestEvent::Positive(1, 10, 20, 2));

        apply_one(&mut history, TestEvent::Negative(1, 10, 10, 1));

        assert_eq!(history.inputs.storage_counts(), (1, 1));
        assert_eq!(history.snapshot_only_at(10).items, vec![-1, 2]);
    }

    #[test]
    fn batch_after_apply_observes_final_bucket_state() {
        let snapshot = ContextSnapshot { id: 1, time: 0, sum: 0 };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        let mut context = Vec::new();

        history.apply_input_batch(vec![ContextEvent { id: 20, time: 10, snapshot_id: 1, value: 2 }], &mut context);
        history.apply_input_batch(vec![ContextEvent { id: 10, time: 10, snapshot_id: 1, value: 1 }], &mut context);

        assert_eq!(history.snapshot_only_at(10).sum, 3);
        assert_eq!(context, vec![2, 3]);
    }

    #[derive(Default)]
    struct WrapperTrace {
        batches: Vec<(i64, Vec<i32>)>,
    }

    impl crate::ApplyWrapper<ContextSnapshot> for WrapperTrace {
        fn apply_input_batch_wrapper(
            &mut self,
            batch: InputBatch<'_, ContextEvent>,
            apply_inner: &mut crate::ApplyInner<'_, ContextSnapshot>,
        ) {
            self.batches.push((batch.time, batch.inputs.iter().copied().map(|event| event.value).collect()));
            apply_inner.apply_input_batch(batch);
        }
    }

    #[test]
    fn apply_wrapper_observes_default_single_batch_apply() {
        let snapshot = ContextSnapshot { id: 1, time: 0, sum: 0 };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        let mut context = WrapperTrace::default();

        history.apply_input_batch(vec![ContextEvent { id: 10, time: 100, snapshot_id: 1, value: 7 }], &mut context);

        assert_eq!(history.snapshot_only_at(100).sum, 7);
        assert_eq!(context.batches, vec![(100, vec![7])]);
    }

    #[derive(Default)]
    struct ReconciliationTrace {
        applied_batches: Vec<i64>,
        reconciled_batches: Vec<i64>,
    }

    impl crate::ApplyWrapper<ContextSnapshot> for ReconciliationTrace {
        fn apply_input_batch_wrapper(
            &mut self,
            batch: InputBatch<'_, ContextEvent>,
            apply_inner: &mut crate::ApplyInner<'_, ContextSnapshot>,
        ) {
            self.applied_batches.push(batch.time);
            apply_inner.apply_input_batch(batch);
        }

        fn reconcile_input_batch_wrapper(
            &mut self,
            batch: InputBatch<'_, ContextEvent>,
            apply_inner: &mut crate::ApplyInner<'_, ContextSnapshot>,
        ) {
            self.reconciled_batches.push(batch.time);
            self.apply_input_batch_wrapper(batch, apply_inner);
        }
    }

    #[test]
    fn reconciliation_is_observable_without_changing_historical_reconstruction() {
        let snapshot = ContextSnapshot { id: 1, time: 0, sum: 0 };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        let mut context = ReconciliationTrace::default();

        history.apply_input_batch(vec![ContextEvent { id: 10, time: 10, snapshot_id: 1, value: 7 }], &mut context);
        history.apply_input_batch(vec![ContextEvent { id: 20, time: 20, snapshot_id: 1, value: 11 }], &mut context);
        context.applied_batches.clear();
        context.reconciled_batches.clear();

        let reconstructed =
            history.snapshot_only_at_with_context(10, &mut context).expect("an event at the query time should materialize a snapshot");

        assert_eq!(reconstructed.sum, 7, "historical reconstruction changed snapshot application semantics");
        assert_eq!(context.applied_batches, vec![10], "historical reconstruction did not use the ordinary apply wrapper");
        assert!(context.reconciled_batches.is_empty(), "historical reconstruction invoked the authoritative reconciliation callback");
    }

    #[derive(Default)]
    struct MultiApplyWrapper {
        batches: Vec<(i64, Vec<i32>)>,
    }

    impl crate::ApplyWrapper<ContextSnapshot> for MultiApplyWrapper {
        fn apply_input_batch_wrapper(
            &mut self,
            batch: InputBatch<'_, ContextEvent>,
            apply_inner: &mut crate::ApplyInner<'_, ContextSnapshot>,
        ) {
            let earlier = [ContextEvent { id: 1, time: batch.time - 10, snapshot_id: batch.snapshot_id, value: 3 }];
            let earlier_refs = [&earlier[0]];
            let earlier_batch = InputBatch { snapshot_id: batch.snapshot_id, time: batch.time - 10, inputs: &earlier_refs };

            self.batches.push((earlier_batch.time, earlier_batch.inputs.iter().copied().map(|event| event.value).collect()));
            apply_inner.apply_input_batch(earlier_batch);

            self.batches.push((batch.time, batch.inputs.iter().copied().map(|event| event.value).collect()));
            apply_inner.apply_input_batch(batch);
        }
    }

    #[test]
    fn apply_wrapper_can_call_inner_multiple_times_with_temporary_batch() {
        let snapshot = ContextSnapshot { id: 1, time: 0, sum: 0 };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        let mut context = MultiApplyWrapper::default();

        history.apply_input_batch(vec![ContextEvent { id: 10, time: 100, snapshot_id: 1, value: 7 }], &mut context);

        assert_eq!(history.snapshot_only_at(100).sum, 10);
        assert_eq!(context.batches, vec![(90, vec![3]), (100, vec![7])]);
    }

    // --- advance tests ---

    #[test]
    fn advance_retains_next_event_in_replay_anchor_at_event_time() {
        let base = TestSnapshot { id: 1, time: 20, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 20, 0);
        apply_one(&mut history, TestEvent::Positive(1, 30, 30, 5));

        history.advance(40);

        let anchor = first_checkpoint(&history).expect("replay anchor");
        assert_eq!(anchor.sum, 5);
        assert_eq!(anchor.items, vec![5]);
        assert_eq!(anchor.time, 30);
        assert!(history.inputs.is_empty());
    }

    #[test]
    fn advance_folds_multiple_events_into_replay_anchor_in_order() {
        let base = TestSnapshot { id: 1, time: 20, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 20, 0);
        apply_one(&mut history, TestEvent::Positive(1, 30, 30, 5));
        apply_one(&mut history, TestEvent::Positive(1, 35, 35, 10));
        apply_one(&mut history, TestEvent::Positive(1, 45, 45, 20));

        history.advance(40);

        let anchor = first_checkpoint(&history).expect("replay anchor");
        assert_eq!(anchor.sum, 15);
        assert_eq!(anchor.items, vec![5, 10]);
        assert_eq!(anchor.time, 35);
        assert_eq!(history.inputs.entries().map(|(time, _id, _input)| time).collect::<Vec<_>>(), vec![45]);
        assert_eq!(history.snapshot_only_at(45).items, vec![5, 10, 20]);
    }

    #[test]
    fn same_time_input_after_pruning_replays_from_the_retained_anchor() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 0);

        apply_one(&mut history, TestEvent::Positive(1, 15, 15, 12));
        history.advance(20);
        apply_one(&mut history, TestEvent::Positive(1, 25, 25, 3));
        assert_eq!(history.snapshot_only_at(25).sum, 15);

        apply_one(&mut history, TestEvent::Positive(1, 25, 26, 0));

        assert_eq!(history.snapshot_only_at(25).sum, 15);
    }

    #[test]
    fn advance_without_pruned_events_preserves_initial_checkpoint_time() {
        let base = TestSnapshot { id: 1, time: 20, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 20, 0);

        history.advance(40);

        assert_eq!(first_checkpoint(&history).expect("initial checkpoint").time, 20);
    }

    #[test]
    fn test_advance_drops_old_events() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 100, 50);
        // lower_time_horizon_delta=50, current_time starts at 100
        // drop_time = current_time - 50

        // Add events at times 40, 60, 80
        apply_one(&mut history, TestEvent::Positive(1, 40, 40, 1));
        apply_one(&mut history, TestEvent::Positive(1, 60, 60, 2));
        apply_one(&mut history, TestEvent::Positive(1, 80, 80, 3));

        // Advance to 120, making drop_time = 70.
        // Events at t=40 should be dropped, t=60 is also < 70 so dropped
        let delta = history.advance(120);
        assert!(delta < 0);
        assert_eq!(history.inputs.len(), 1); // only t=80 remains
    }

    #[test]
    fn advance_prunes_ordered_and_late_inputs_after_folding_them_into_the_anchor() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 0);
        apply_one(&mut history, TestEvent::Positive(1, 10, 10, 1));
        apply_one(&mut history, TestEvent::Positive(1, 30, 30, 3));
        apply_one(&mut history, TestEvent::Positive(1, 20, 20, 2));
        assert_eq!(history.inputs.storage_counts(), (2, 1));

        history.advance(40);

        assert!(history.inputs.is_empty());
        assert_eq!(first_checkpoint(&history).expect("replay anchor").items, vec![1, 2, 3]);
    }

    #[test]
    fn test_advance_promotes_checkpoint_to_replay_anchor() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 50);
        history.checkpoint_interval = 1; // checkpoint every event

        // Add events so checkpoints are created
        apply_one(&mut history, TestEvent::Positive(1, 10, 10, 5));
        apply_one(&mut history, TestEvent::Positive(1, 20, 20, 10));
        apply_one(&mut history, TestEvent::Positive(1, 30, 30, 15));

        // Advance to 80, making drop_time = 30.
        // Events at t=10, t=20 dropped. Checkpoint at t=20 becomes base.
        history.advance(80);

        assert_eq!(first_checkpoint(&history).expect("replay anchor").sum, 15); // 5 + 10
        assert_eq!(history.inputs.len(), 1); // only t=30 remains
    }

    #[test]
    fn test_advance_noop() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);

        // No events, advance should be no-op
        let delta = history.advance(10);
        assert_eq!(delta, 0);
        assert!(history.inputs.is_empty());
        assert_eq!(history.checkpoints.len(), 1);
    }
}
