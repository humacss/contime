use std::collections::{BTreeMap, VecDeque};

use crate::{ContimeKey, ContimeTime, Input, InputLanes, Snapshot, SnapshotLanes};

use super::checkpoints::{get_checkpoint_at, get_checkpoint_at_with_context, get_checkpoint_before_with_context};

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
    pub checkpoints: VecDeque<(ContimeKey<S::Time>, S)>,
    /// Routed temporal inputs keyed by time and id.
    pub inputs: BTreeMap<ContimeKey<S::Time>, S::Input>,
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
        let snapshot_size = snapshot.conservative_size() as i64;
        let checkpoint_key = ContimeKey { time: snapshot.time(), id: u128::MAX };
        let mut checkpoints = VecDeque::new();
        checkpoints.push_back((checkpoint_key, snapshot));
        (
            Self {
                current_time,
                lower_time_horizon_delta,
                snapshot_id,
                snapshot_lane_index: Some(snapshot_lane_index),
                checkpoints,
                inputs: BTreeMap::new(),
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
                inputs: BTreeMap::new(),
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

        let replay_anchor = get_checkpoint_before_with_context(self, drop_time, context);

        let first_kept_checkpoint = self.checkpoints.partition_point(|(key, _checkpoint)| key < &drop_key);
        for (_key, checkpoint) in self.checkpoints.drain(..first_kept_checkpoint) {
            bytes_delta -= checkpoint.conservative_size() as i64;
        }

        if let Some((key, checkpoint)) = replay_anchor {
            if self.checkpoints.front().is_none_or(|(existing_key, _checkpoint)| existing_key > &key) {
                bytes_delta += checkpoint.conservative_size() as i64;
                self.checkpoints.push_front((key, checkpoint));
            }
        }

        let kept_inputs = self.inputs.split_off(&drop_key);
        for input in self.inputs.values() {
            bytes_delta -= input.conservative_size() as i64;
        }
        self.inputs = kept_inputs;

        bytes_delta
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

    use crate::{ApplyBatch, Event, Input, InputBatch, InputLanes, SnapshotEvent, TestEvent, TestSnapshot};

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
            snapshot: &mut ContextSnapshot,
            batch: InputBatch<'_, ContextEvent>,
            apply_inner: crate::ApplyInner<ContextSnapshot>,
        ) {
            apply_inner.apply_input_batch(snapshot, batch);
            self.push(snapshot.sum);
        }
    }

    fn checkpoint_keys<S: Snapshot>(history: &SnapshotHistory<S>) -> Vec<ContimeKey<S::Time>> {
        history.checkpoints.iter().map(|(key, _checkpoint)| key.clone()).collect()
    }

    fn checkpoint<'a, S: Snapshot>(history: &'a SnapshotHistory<S>, key: &ContimeKey<S::Time>) -> Option<&'a S> {
        history.checkpoints.iter().find(|(checkpoint_key, _checkpoint)| checkpoint_key == key).map(|(_key, checkpoint)| checkpoint)
    }

    fn first_checkpoint<S: Snapshot>(history: &SnapshotHistory<S>) -> Option<&S> {
        history.checkpoints.front().map(|(_key, checkpoint)| checkpoint)
    }

    fn apply_one<S>(history: &mut SnapshotHistory<S>, event: S::Input)
    where
        S: crate::SnapshotLanes + 'static,
        S::Input: InputLanes<S>,
    {
        history.apply_input_batch(vec![event], &mut ());
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
            snapshot: &mut ContextSnapshot,
            batch: InputBatch<'_, ContextEvent>,
            apply_inner: crate::ApplyInner<ContextSnapshot>,
        ) {
            self.batches.push((batch.time, batch.inputs.iter().copied().map(|event| event.value).collect()));
            apply_inner.apply_input_batch(snapshot, batch);
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
    struct MultiApplyWrapper {
        batches: Vec<(i64, Vec<i32>)>,
    }

    impl crate::ApplyWrapper<ContextSnapshot> for MultiApplyWrapper {
        fn apply_input_batch_wrapper(
            &mut self,
            snapshot: &mut ContextSnapshot,
            batch: InputBatch<'_, ContextEvent>,
            apply_inner: crate::ApplyInner<ContextSnapshot>,
        ) {
            let earlier = [ContextEvent { id: 1, time: batch.time - 10, snapshot_id: batch.snapshot_id, value: 3 }];
            let earlier_refs = [&earlier[0]];
            let earlier_batch = InputBatch { snapshot_id: batch.snapshot_id, time: batch.time - 10, inputs: &earlier_refs };

            self.batches.push((earlier_batch.time, earlier_batch.inputs.iter().copied().map(|event| event.value).collect()));
            apply_inner.apply_input_batch(snapshot, earlier_batch);

            self.batches.push((batch.time, batch.inputs.iter().copied().map(|event| event.value).collect()));
            apply_inner.apply_input_batch(snapshot, batch);
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
        assert_eq!(history.inputs.keys().map(|key| key.time).collect::<Vec<_>>(), vec![45]);
        assert_eq!(history.snapshot_only_at(45).items, vec![5, 10, 20]);
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
