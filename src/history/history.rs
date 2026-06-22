use std::collections::{BTreeMap, VecDeque};

use crate::{ApplyEvents, ContimeKey, Event, Snapshot};

use super::checkpoints::get_checkpoint_at;

type SnapshotId = u128;

/// Advanced per-snapshot history store used internally by `Contime`.
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
    /// Base snapshot used when replay starts before the first checkpoint.
    pub base_snapshot: S,
    /// Materialized checkpoints sorted by event time and id.
    pub checkpoints: VecDeque<(ContimeKey, S)>,
    /// Applied events keyed by time and id.
    pub events: BTreeMap<ContimeKey, S::Event>,
    /// Interval between generated checkpoints during replay.
    pub checkpoint_interval: usize,

    current_time: i64,
    lower_time_horizon_delta: i64,
}

const CHECKPOINT_INTERVAL: usize = 100;

impl<S> LocalSnapshotHistory<S>
where
    S: Snapshot + ApplyEvents + 'static,
{
    /// Creates a history for one snapshot and returns it with its initial memory cost.
    pub fn new(snapshot: S, current_time: i64, lower_time_horizon_delta: i64) -> (Self, i64) {
        Self::new_with_snapshot_id(snapshot.id(), snapshot, current_time, lower_time_horizon_delta)
    }

    /// Creates a history for one explicitly routed snapshot id.
    pub fn new_with_snapshot_id(snapshot_id: u128, snapshot: S, current_time: i64, lower_time_horizon_delta: i64) -> (Self, i64) {
        let checkpoints = VecDeque::new();
        let events = BTreeMap::new();
        let mut base_snapshot = snapshot.clone();
        base_snapshot.set_time(0);

        let base_size = base_snapshot.conservative_size() as i64;

        (
            Self {
                current_time,
                lower_time_horizon_delta,
                snapshot_id,
                base_snapshot,
                checkpoints,
                events,
                checkpoint_interval: CHECKPOINT_INTERVAL,
            },
            base_size,
        )
    }

    /// Advances the internal current time and prunes history outside the configured horizon.
    pub fn advance(&mut self, time: i64) -> i64 {
        self.current_time += time;
        let drop_time = self.current_time - self.lower_time_horizon_delta;

        let drop_key = ContimeKey { time: drop_time, id: u128::MIN };

        let mut bytes_delta: i64 = 0;

        // Split off events and checkpoints at the drop boundary
        let kept_events = self.events.split_off(&drop_key);
        for (_key, event) in &self.events {
            bytes_delta -= event.conservative_size() as i64;
        }

        while self.checkpoints.len() > 1 {
            let second_key = &self.checkpoints[1].0;
            if second_key >= &drop_key {
                break;
            }
            let (_key, checkpoint) = self.checkpoints.pop_front().expect("checkpoint length checked");
            self.base_snapshot = checkpoint.clone();
            bytes_delta -= checkpoint.conservative_size() as i64;
        }
        if let Some((key, checkpoint)) = self.checkpoints.front() {
            if key < &drop_key {
                self.base_snapshot = checkpoint.clone();
            }
        }

        self.events = kept_events;

        bytes_delta
    }

    /// Reconstructs the snapshot state at `time`.
    ///
    /// Events at exactly `time` are included in the returned snapshot.
    pub fn snapshot_at(&self, time: i64) -> S {
        get_checkpoint_at(self, time)
    }

    /// Reconstructs the snapshot state at `time`.
    ///
    /// Events at exactly `time` are included in the returned snapshot.
    pub fn snapshot_only_at(&self, time: i64) -> S {
        get_checkpoint_at(self, time)
    }
}

/// Concrete per-snapshot history type used by the crate.
pub type SnapshotHistory<S> = LocalSnapshotHistory<S>;

#[cfg(test)]
mod tests {
    #![allow(unused_must_use)]

    use super::*;

    use crate::{ApplyBatch, ApplyEvents, Event, SnapshotEvent, TestEvent, TestSnapshot};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ContextEvent {
        id: u128,
        time: i64,
        snapshot_id: u128,
        value: i32,
    }

    impl Event for ContextEvent {
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

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ContextSnapshot {
        id: u128,
        time: i64,
        sum: i32,
    }

    impl Snapshot for ContextSnapshot {
        type Event = ContextEvent;

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

        fn from_event(event: &Self::Event) -> Self {
            Self { id: event.snapshot_id, time: event.time, sum: 0 }
        }
    }

    impl SnapshotEvent<ContextSnapshot> for ContextEvent {
        fn snapshot_id(&self) -> u128 {
            self.snapshot_id
        }
    }

    impl ApplyEvents for ContextSnapshot {
        fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Event>) {
            self.id = batch.snapshot_id;
            for event in batch.events.iter().copied() {
                self.sum += event.value;
            }
            self.set_time(batch.time);
        }
    }

    impl crate::ApplyWrapper<ContextSnapshot> for Vec<i32> {
        type Error = std::convert::Infallible;

        fn apply_event_batch_wrapper(
            &mut self,
            snapshot: &mut ContextSnapshot,
            batch: ApplyBatch<'_, ContextEvent>,
            apply_inner: crate::ApplyInner<ContextSnapshot>,
        ) -> Result<(), Self::Error> {
            apply_inner.apply_event_batch(snapshot, batch);
            self.push(snapshot.sum);
            Ok(())
        }
    }

    fn checkpoint_keys<S: Snapshot>(history: &SnapshotHistory<S>) -> Vec<ContimeKey> {
        history.checkpoints.iter().map(|(key, _checkpoint)| key.clone()).collect()
    }

    fn checkpoint<'a, S: Snapshot>(history: &'a SnapshotHistory<S>, key: &ContimeKey) -> Option<&'a S> {
        history.checkpoints.iter().find(|(checkpoint_key, _checkpoint)| checkpoint_key == key).map(|(_key, checkpoint)| checkpoint)
    }

    fn first_checkpoint<S: Snapshot>(history: &SnapshotHistory<S>) -> Option<&S> {
        history.checkpoints.front().map(|(_key, checkpoint)| checkpoint)
    }

    fn apply_one<S>(history: &mut SnapshotHistory<S>, event: S::Event)
    where
        S: Snapshot + ApplyEvents + 'static,
    {
        history.apply_event_batch(vec![event], &mut ()).unwrap();
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

        history.apply_event_batch(vec![ContextEvent { id: 20, time: 10, snapshot_id: 1, value: 2 }], &mut context);
        history.apply_event_batch(vec![ContextEvent { id: 10, time: 10, snapshot_id: 1, value: 1 }], &mut context);

        assert_eq!(history.snapshot_only_at(10).sum, 3);
        assert_eq!(context, vec![2, 3]);
    }

    #[derive(Default)]
    struct WrapperTrace {
        batches: Vec<(i64, Vec<i32>)>,
    }

    impl crate::ApplyWrapper<ContextSnapshot> for WrapperTrace {
        type Error = std::convert::Infallible;

        fn apply_event_batch_wrapper(
            &mut self,
            snapshot: &mut ContextSnapshot,
            batch: ApplyBatch<'_, ContextEvent>,
            apply_inner: crate::ApplyInner<ContextSnapshot>,
        ) -> Result<(), Self::Error> {
            self.batches.push((batch.time, batch.events.iter().copied().map(|event| event.value).collect()));
            apply_inner.apply_event_batch(snapshot, batch);
            Ok(())
        }
    }

    #[test]
    fn apply_wrapper_observes_default_single_batch_apply() {
        let snapshot = ContextSnapshot { id: 1, time: 0, sum: 0 };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        let mut context = WrapperTrace::default();

        history.apply_event_batch(vec![ContextEvent { id: 10, time: 100, snapshot_id: 1, value: 7 }], &mut context);

        assert_eq!(history.snapshot_only_at(100).sum, 7);
        assert_eq!(context.batches, vec![(100, vec![7])]);
    }

    #[derive(Default)]
    struct MultiApplyWrapper {
        batches: Vec<(i64, Vec<i32>)>,
    }

    impl crate::ApplyWrapper<ContextSnapshot> for MultiApplyWrapper {
        type Error = std::convert::Infallible;

        fn apply_event_batch_wrapper(
            &mut self,
            snapshot: &mut ContextSnapshot,
            batch: ApplyBatch<'_, ContextEvent>,
            apply_inner: crate::ApplyInner<ContextSnapshot>,
        ) -> Result<(), Self::Error> {
            let earlier = [ContextEvent { id: 1, time: batch.time - 10, snapshot_id: batch.snapshot_id, value: 3 }];
            let earlier_refs = [&earlier[0]];
            let earlier_batch = ApplyBatch { snapshot_id: batch.snapshot_id, time: batch.time - 10, events: &earlier_refs };

            self.batches.push((earlier_batch.time, earlier_batch.events.iter().copied().map(|event| event.value).collect()));
            apply_inner.apply_event_batch(snapshot, earlier_batch);

            self.batches.push((batch.time, batch.events.iter().copied().map(|event| event.value).collect()));
            apply_inner.apply_event_batch(snapshot, batch);
            Ok(())
        }
    }

    #[test]
    fn apply_wrapper_can_call_inner_multiple_times_with_temporary_batch() {
        let snapshot = ContextSnapshot { id: 1, time: 0, sum: 0 };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        let mut context = MultiApplyWrapper::default();

        history.apply_event_batch(vec![ContextEvent { id: 10, time: 100, snapshot_id: 1, value: 7 }], &mut context).unwrap();

        assert_eq!(history.snapshot_only_at(100).sum, 10);
        assert_eq!(context.batches, vec![(90, vec![3]), (100, vec![7])]);
    }

    #[derive(Default)]
    struct EarlierExtraBatchWrapper {
        observed_before_apply: Vec<(i64, i32)>,
    }

    impl crate::ApplyWrapper<ContextSnapshot> for EarlierExtraBatchWrapper {
        type Error = std::convert::Infallible;

        fn extra_batches(&mut self, batch: ApplyBatch<'_, ContextEvent>) -> Result<Vec<Vec<ContextEvent>>, Self::Error> {
            if batch.time == 100 {
                Ok(vec![vec![ContextEvent { id: 1, time: 90, snapshot_id: batch.snapshot_id, value: 3 }]])
            } else {
                Ok(Vec::new())
            }
        }

        fn apply_event_batch_wrapper(
            &mut self,
            snapshot: &mut ContextSnapshot,
            batch: ApplyBatch<'_, ContextEvent>,
            apply_inner: crate::ApplyInner<ContextSnapshot>,
        ) -> Result<(), Self::Error> {
            self.observed_before_apply.push((batch.time, snapshot.sum));
            apply_inner.apply_event_batch(snapshot, batch);
            Ok(())
        }
    }

    #[test]
    fn apply_wrapper_extra_batches_recompute_from_the_earliest_extra_batch() {
        let snapshot = ContextSnapshot { id: 1, time: 0, sum: 0 };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        apply_one(&mut history, ContextEvent { id: 95, time: 95, snapshot_id: 1, value: 5 });

        let mut context = EarlierExtraBatchWrapper::default();
        history.apply_event_batch(vec![ContextEvent { id: 100, time: 100, snapshot_id: 1, value: 7 }], &mut context).unwrap();

        assert_eq!(context.observed_before_apply, vec![(90, 0), (95, 3), (100, 8)]);
        assert_eq!(history.snapshot_only_at(100).sum, 15);
    }

    #[derive(Default)]
    struct FailingWrapper;

    impl crate::ApplyWrapper<ContextSnapshot> for FailingWrapper {
        type Error = crate::ApplyError;

        fn apply_event_batch_wrapper(
            &mut self,
            _snapshot: &mut ContextSnapshot,
            batch: ApplyBatch<'_, ContextEvent>,
            _apply_inner: crate::ApplyInner<ContextSnapshot>,
        ) -> Result<(), Self::Error> {
            Err(crate::ApplyError::new(format!("rejected batch at {}", batch.time)))
        }
    }

    #[test]
    fn apply_wrapper_errors_are_returned_to_apply_caller() {
        let snapshot = ContextSnapshot { id: 1, time: 0, sum: 0 };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        let mut context = FailingWrapper;

        let error =
            history.apply_event_batch(vec![ContextEvent { id: 10, time: 100, snapshot_id: 1, value: 7 }], &mut context).unwrap_err();

        assert_eq!(error.message(), "rejected batch at 100");
    }

    // --- advance tests ---

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

        // Advance by 20 → current_time = 120, drop_time = 70
        // Events at t=40 should be dropped, t=60 is also < 70 so dropped
        let delta = history.advance(20);
        assert!(delta < 0);
        assert_eq!(history.events.len(), 1); // only t=80 remains
    }

    #[test]
    fn test_advance_promotes_checkpoint_to_base() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 50);
        history.checkpoint_interval = 1; // checkpoint every event

        // Add events so checkpoints are created
        apply_one(&mut history, TestEvent::Positive(1, 10, 10, 5));
        apply_one(&mut history, TestEvent::Positive(1, 20, 20, 10));
        apply_one(&mut history, TestEvent::Positive(1, 30, 30, 15));

        // current_time=0, advance by 80 → current_time=80, drop_time=30
        // Events at t=10, t=20 dropped. Checkpoint at t=20 becomes base.
        history.advance(80);

        assert_eq!(history.base_snapshot.sum, 15); // 5 + 10
        assert_eq!(history.events.len(), 1); // only t=30 remains
    }

    #[test]
    fn test_advance_noop() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);

        // No events, advance should be no-op
        let delta = history.advance(10);
        assert_eq!(delta, 0);
        assert!(history.events.is_empty());
        assert!(history.checkpoints.is_empty());
    }
}
