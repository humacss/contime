use std::collections::BTreeMap;
use std::ops::Bound;

use flume::{bounded, Receiver, Sender, TrySendError};

use crate::{AfterApplyEvents, ApplyEvents, ContimeKey, Event, Snapshot};

use super::apply::replay_and_checkpoint;

type SnapshotId = u128;

/// Notification sent when previously queried history must be reconsidered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    /// Snapshot id whose historical state changed.
    pub snapshot_id: u128,
    /// Earliest time that may have changed.
    pub from_time: i64,
    /// Latest previously known event time affected by the change.
    pub to_time: i64,
}

/// Outcome from applying an event or authoritative snapshot to one history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// Signed memory delta from the apply.
    pub bytes_delta: i64,
}

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
    /// Materialized checkpoints keyed by event time and id.
    pub checkpoints: BTreeMap<ContimeKey, S>,
    /// Applied events keyed by time and id.
    pub events: BTreeMap<ContimeKey, S::Event>,
    /// Interval between generated checkpoints during replay.
    pub checkpoint_interval: usize,

    current_time: i64,
    lower_time_horizon_delta: i64,

    reconciliation_subscribers: Vec<Sender<Reconciliation>>,
}

const CHECKPOINT_INTERVAL: usize = 100;

impl<S> LocalSnapshotHistory<S>
where
    S: Snapshot + ApplyEvents + 'static,
{
    /// Creates a history for one snapshot and returns it with its initial memory cost.
    pub fn new(snapshot: S, current_time: i64, lower_time_horizon_delta: i64) -> (Self, i64) {
        let checkpoints = BTreeMap::new();
        let events = BTreeMap::new();
        let mut base_snapshot = snapshot.clone();
        base_snapshot.set_time(0);

        let base_size = base_snapshot.conservative_size() as i64;

        (
            Self {
                current_time,
                lower_time_horizon_delta,
                reconciliation_subscribers: vec![],
                snapshot_id: snapshot.id(),
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

        // Update base_snapshot to the latest dropped checkpoint before replacing
        let kept_checkpoints = self.checkpoints.split_off(&drop_key);
        if let Some((_key, last_dropped)) = self.checkpoints.iter().next_back() {
            self.base_snapshot = last_dropped.clone();
        }
        for (_key, checkpoint) in &self.checkpoints {
            bytes_delta -= checkpoint.conservative_size() as i64;
        }

        self.events = kept_events;
        self.checkpoints = kept_checkpoints;

        bytes_delta
    }

    /// Applies an event to this history and returns the memory delta.
    pub fn apply_event(&mut self, event: S::Event) -> ApplyOutcome
    where
        S: AfterApplyEvents<()>,
    {
        self.apply_event_with_context(event, &mut ())
    }

    /// Applies an event with an optional runtime context.
    ///
    /// Context effects are produced only while handling this explicit apply.
    /// Replay and query materialization continue to use context-free state
    /// application.
    pub fn apply_event_with_context<C>(&mut self, event: S::Event, context: &mut C) -> ApplyOutcome
    where
        S: AfterApplyEvents<C>,
    {
        let event_time = event.time();
        let latest_event_key = self.events.keys().next_back().cloned();

        let event_key = ContimeKey::from_event(&event);
        if self.events.get(&event_key).is_some_and(|existing| existing == &event) {
            return ApplyOutcome { bytes_delta: 0 };
        }

        let is_new_latest_time = latest_event_key.as_ref().is_none_or(|latest| event_key.time > latest.time);
        let mut bytes_delta = event.conservative_size() as i64;
        self.events.insert(event_key.clone(), event);
        bytes_delta += self.replay_from_bucket(event_time, is_new_latest_time, context);

        // Send reconciliation if this apply changed already-observed history.
        if let Some(latest_key) = latest_event_key {
            if event_key < latest_key {
                let reconciliation = Reconciliation { snapshot_id: self.snapshot_id, from_time: event_time, to_time: latest_key.time };
                self.notify_reconciliation(reconciliation.clone());
                return ApplyOutcome { bytes_delta };
            }
        }

        ApplyOutcome { bytes_delta }
    }

    fn replay_from_bucket<C>(&mut self, time: i64, preserve_previous_tip: bool, context: &mut C) -> i64
    where
        S: AfterApplyEvents<C>,
    {
        let mut replay_boundary = first_key_at_time(time);
        if !preserve_previous_tip {
            if let Some((key, _checkpoint)) = self.checkpoints.range(..&replay_boundary).next_back() {
                if !self.is_cadence_checkpoint(key) {
                    replay_boundary = key.clone();
                }
            }
        }

        let checkpoint_entry = self.checkpoints.range(..&replay_boundary).next_back();
        let remove_replay_checkpoint =
            preserve_previous_tip && checkpoint_entry.as_ref().is_some_and(|(key, _checkpoint)| !self.is_cadence_checkpoint(key));
        let (mut snapshot, replay_start) = match checkpoint_entry {
            Some((key, checkpoint)) => (checkpoint.clone(), Bound::Excluded(key.clone())),
            None => (self.base_snapshot.clone(), Bound::Unbounded),
        };

        let stale_keys = self.checkpoints.range(replay_boundary.clone()..).map(|(key, _)| key.clone()).collect::<Vec<_>>();
        let mut bytes_delta = 0;
        if remove_replay_checkpoint {
            if let Some(key) = self.checkpoints.range(..&replay_boundary).next_back().map(|(key, _)| key.clone()) {
                if let Some(removed) = self.checkpoints.remove(&key) {
                    bytes_delta -= removed.conservative_size() as i64;
                }
            }
        }
        for key in stale_keys {
            if let Some(removed) = self.checkpoints.remove(&key) {
                bytes_delta -= removed.conservative_size() as i64;
            }
        }

        let replay_events =
            self.events.range((replay_start, Bound::Unbounded)).map(|(key, event)| (key.clone(), event.clone())).collect::<Vec<_>>();
        let mut event_count = 0usize;
        let mut index = 0usize;
        let mut latest_key = None;

        while index < replay_events.len() {
            let bucket_time = replay_events[index].0.time;
            let mut bucket = Vec::new();
            let mut bucket_last_key = replay_events[index].0.clone();

            while index < replay_events.len() && replay_events[index].0.time == bucket_time {
                bucket_last_key = replay_events[index].0.clone();
                bucket.push(replay_events[index].1.clone());
                index += 1;
            }

            <S as ApplyEvents>::apply_events(&mut snapshot, bucket_time, &bucket);
            snapshot.set_time(bucket_time);
            <S as AfterApplyEvents<C>>::after_apply_events(&snapshot, bucket_time, &bucket, context);
            event_count += bucket.len();
            latest_key = Some(bucket_last_key.clone());

            if self.checkpoint_interval != 0 && event_count % self.checkpoint_interval == 0 {
                bytes_delta += snapshot.conservative_size() as i64;
                self.checkpoints.insert(bucket_last_key, snapshot.clone());
            }
        }

        if let Some(latest_key) = latest_key {
            if self.checkpoints.keys().next_back() != Some(&latest_key) {
                bytes_delta += snapshot.conservative_size() as i64;
                self.checkpoints.insert(latest_key, snapshot);
            }
        }

        bytes_delta
    }

    fn is_cadence_checkpoint(&self, checkpoint_key: &ContimeKey) -> bool {
        if self.checkpoint_interval == 0 {
            return true;
        }

        let previous_checkpoint = self.checkpoints.range(..checkpoint_key).next_back().map(|(key, _)| key.clone());
        let replay_start = previous_checkpoint.as_ref().map_or(Bound::Unbounded, Bound::Excluded);
        let event_count = self.events.range((replay_start, Bound::Included(checkpoint_key))).count();

        event_count != 0 && event_count % self.checkpoint_interval == 0
    }

    /// Applies an authoritative snapshot and replays later events on top of it.
    pub fn apply_snapshot(&mut self, snapshot: S) -> ApplyOutcome {
        if snapshot.id() != self.snapshot_id {
            return ApplyOutcome { bytes_delta: 0 };
        }

        let snapshot_key = last_key_at_time(snapshot.time());
        let stale_boundary = first_key_at_time(snapshot.time());

        let mut bytes_delta: i64 = 0;

        // Remove all checkpoints at or after the snapshot time.
        let stale_keys: Vec<ContimeKey> = self.checkpoints.range(stale_boundary..).map(|(k, _)| k.clone()).collect();
        for key in &stale_keys {
            let removed = self.checkpoints.remove(key).expect("stale checkpoint key must exist");
            bytes_delta -= removed.conservative_size() as i64;
        }

        // Insert the snapshot as a checkpoint
        bytes_delta += snapshot.conservative_size() as i64;
        self.checkpoints.insert(snapshot_key.clone(), snapshot.clone());

        // Replay only events strictly after the snapshot time to rebuild downstream checkpoints.
        bytes_delta += replay_and_checkpoint(
            &snapshot,
            Bound::Excluded(&last_key_at_time(snapshot.time())),
            &mut self.checkpoints,
            &self.events,
            self.checkpoint_interval,
        );

        // Send reconciliation notification
        let latest_event_time = self.events.keys().next_back().map(|k| k.time).unwrap_or(snapshot.time()).max(snapshot.time());
        let reconciliation = Reconciliation { snapshot_id: self.snapshot_id, from_time: snapshot.time(), to_time: latest_event_time };
        self.notify_reconciliation(reconciliation.clone());

        ApplyOutcome { bytes_delta }
    }

    /// Reconstructs the snapshot state at `time` and returns a reconciliation receiver.
    ///
    /// Events at exactly `time` are included in the returned snapshot.
    pub fn snapshot_at(&mut self, time: i64) -> (S, Receiver<Reconciliation>) {
        (self.materialize_snapshot_at(time), self.subscribe_reconciliation())
    }

    /// Reconstructs the snapshot state at `time` without creating a reconciliation receiver.
    ///
    /// Events at exactly `time` are included in the returned snapshot.
    pub fn snapshot_only_at(&self, time: i64) -> S {
        self.materialize_snapshot_at(time)
    }

    /// Returns retained events where `from_time <= event.time() < to_time`.
    ///
    /// Results are sorted by `(time, event_id)`, matching the internal event
    /// key order. Events already pruned by the configured history horizon are
    /// not returned.
    pub fn events_between(&self, from_time: i64, to_time: i64) -> Vec<S::Event>
    where
        S::Event: Clone,
    {
        if from_time >= to_time {
            return Vec::new();
        }

        self.events.range(first_key_at_time(from_time)..first_key_at_time(to_time)).map(|(_key, event)| event.clone()).collect()
    }

    fn materialize_snapshot_at(&self, time: i64) -> S {
        let checkpoint_boundary = last_key_at_time(time);

        // Find the latest checkpoint at or before the query time.
        let checkpoint_entry = self.checkpoints.range(..=checkpoint_boundary).next_back();

        let (mut snapshot, replay_start) = match checkpoint_entry {
            Some((key, checkpoint)) => (checkpoint.clone(), Bound::Excluded(key.clone())),
            None => (self.base_snapshot.clone(), Bound::Unbounded),
        };

        let end_key = last_key_at_time(time);

        let replay_events = self
            .events
            .range((replay_start, Bound::Included(end_key)))
            .map(|(key, event)| (key.clone(), event.clone()))
            .collect::<Vec<_>>();
        let mut index = 0usize;
        while index < replay_events.len() {
            let bucket_time = replay_events[index].0.time;
            let mut bucket = Vec::new();

            while index < replay_events.len() && replay_events[index].0.time == bucket_time {
                bucket.push(replay_events[index].1.clone());
                index += 1;
            }

            snapshot.apply_events(bucket_time, &bucket);
            snapshot.set_time(bucket_time);
        }

        snapshot.set_time(time);

        snapshot
    }

    fn subscribe_reconciliation(&mut self) -> Receiver<Reconciliation> {
        self.reconciliation_subscribers.retain(|sender| !sender.is_disconnected());

        let (tx, rx) = bounded(1000);
        self.reconciliation_subscribers.push(tx);
        rx
    }

    fn notify_reconciliation(&mut self, reconciliation: Reconciliation) {
        let mut index = 0;
        while index < self.reconciliation_subscribers.len() {
            match self.reconciliation_subscribers[index].try_send(reconciliation.clone()) {
                Ok(()) | Err(TrySendError::Full(_)) => {
                    index += 1;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.reconciliation_subscribers.swap_remove(index);
                }
            }
        }
    }
}

fn first_key_at_time(time: i64) -> ContimeKey {
    ContimeKey { time, id: u128::MIN }
}

fn last_key_at_time(time: i64) -> ContimeKey {
    ContimeKey { time, id: u128::MAX }
}

/// Concrete per-snapshot history type used by the crate.
pub type SnapshotHistory<S> = LocalSnapshotHistory<S>;

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{AfterApplyEvents, ApplyEvents, Event, SnapshotEvent, TestEvent, TestSnapshot};

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
        fn apply_events(&mut self, time: i64, events: &[Self::Event]) {
            for event in events {
                self.sum += event.value;
            }
            self.set_time(time);
        }
    }

    impl AfterApplyEvents<Vec<i32>> for ContextSnapshot {
        fn after_apply_events(&self, _time: i64, _events: &[Self::Event], context: &mut Vec<i32>) {
            context.push(self.sum);
        }
    }

    #[test]
    fn in_order_apply_updates_current_end_checkpoint_every_time() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        history.checkpoint_interval = 100;

        history.apply_event(TestEvent::Positive(1, 1, 1, 5));
        assert_eq!(history.checkpoints.keys().cloned().collect::<Vec<_>>(), vec![ContimeKey { time: 1, id: 1 }]);
        assert_eq!(history.checkpoints.values().next().expect("checkpoint").sum, 5);

        history.apply_event(TestEvent::Positive(1, 2, 2, 7));
        assert_eq!(history.checkpoints.keys().cloned().collect::<Vec<_>>(), vec![ContimeKey { time: 2, id: 2 }]);
        assert_eq!(history.checkpoints.values().next().expect("checkpoint").sum, 12);
    }

    #[test]
    fn in_order_apply_preserves_cadence_anchor_and_moves_current_end_checkpoint() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        history.checkpoint_interval = 2;

        history.apply_event(TestEvent::Positive(1, 1, 1, 5));
        history.apply_event(TestEvent::Positive(1, 2, 2, 7));
        history.apply_event(TestEvent::Positive(1, 3, 3, 11));

        assert_eq!(
            history.checkpoints.keys().cloned().collect::<Vec<_>>(),
            vec![ContimeKey { time: 2, id: 2 }, ContimeKey { time: 3, id: 3 }]
        );
        assert_eq!(history.checkpoints.get(&ContimeKey { time: 2, id: 2 }).expect("anchor").sum, 12);
        assert_eq!(history.checkpoints.get(&ContimeKey { time: 3, id: 3 }).expect("tip").sum, 23);
    }

    #[test]
    fn out_of_order_apply_overwrites_later_checkpoints_without_deleting_them() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        history.checkpoint_interval = 2;

        history.apply_event(TestEvent::Positive(1, 10, 10, 10));
        history.apply_event(TestEvent::Positive(1, 20, 20, 20));
        history.apply_event(TestEvent::Positive(1, 30, 30, 30));
        assert_eq!(
            history.checkpoints.keys().cloned().collect::<Vec<_>>(),
            vec![ContimeKey { time: 20, id: 20 }, ContimeKey { time: 30, id: 30 }]
        );

        history.apply_event(TestEvent::Positive(1, 15, 15, 15));

        assert_eq!(
            history.checkpoints.keys().cloned().collect::<Vec<_>>(),
            vec![ContimeKey { time: 15, id: 15 }, ContimeKey { time: 30, id: 30 }]
        );
        assert_eq!(history.checkpoints.get(&ContimeKey { time: 15, id: 15 }).expect("checkpoint").sum, 25);
        assert_eq!(history.checkpoints.get(&ContimeKey { time: 30, id: 30 }).expect("tip").sum, 75);
    }

    #[test]
    fn snapshot_at_includes_events_at_query_time() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);

        history.apply_event(TestEvent::Positive(1, 10, 1, 5));

        let actual = history.snapshot_only_at(10);

        assert_eq!(actual.sum, 5);
        assert_eq!(actual.time, 10);
    }

    #[test]
    fn same_millisecond_events_replay_in_event_id_order() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);

        history.apply_event(TestEvent::Positive(1, 10, 20, 2));
        history.apply_event(TestEvent::Negative(1, 10, 10, 1));

        let actual = history.snapshot_only_at(10);

        assert_eq!(actual.items, vec![-1, 2]);
        assert_eq!(actual.sum, 1);
    }

    #[test]
    fn batch_after_apply_observes_final_bucket_state() {
        let snapshot = ContextSnapshot { id: 1, time: 0, sum: 0 };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);
        let mut context = Vec::new();

        history.apply_event_with_context(ContextEvent { id: 20, time: 10, snapshot_id: 1, value: 2 }, &mut context);
        history.apply_event_with_context(ContextEvent { id: 10, time: 10, snapshot_id: 1, value: 1 }, &mut context);

        assert_eq!(history.snapshot_only_at(10).sum, 3);
        assert_eq!(context, vec![2, 3]);
    }

    // --- apply_snapshot tests ---

    #[test]
    fn test_apply_snapshot_wrong_id() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);

        let wrong_snapshot = TestSnapshot { id: 2, time: 5, sum: 10, items: vec![1, 2] };
        let delta = history.apply_snapshot(wrong_snapshot);

        assert_eq!(delta.bytes_delta, 0);
        assert!(history.checkpoints.is_empty());
    }

    #[test]
    fn test_apply_snapshot_no_events() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);

        let auth_snapshot = TestSnapshot { id: 1, time: 5, sum: 10, items: vec![1, 2] };
        let expected_size = auth_snapshot.conservative_size() as i64;
        let delta = history.apply_snapshot(auth_snapshot.clone());

        assert_eq!(delta.bytes_delta, expected_size);
        let key = last_key_at_time(5);
        assert_eq!(history.checkpoints.get(&key), Some(&auth_snapshot));
    }

    #[test]
    fn test_apply_snapshot_between_events() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);
        history.checkpoint_interval = 2;

        // Add events at times 1, 3, 5
        history.apply_event(TestEvent::Positive(1, 1, 1, 10));
        history.apply_event(TestEvent::Positive(1, 3, 3, 20));
        history.apply_event(TestEvent::Positive(1, 5, 5, 30));

        // Apply snapshot at time 2 — should invalidate checkpoints at or after time 2
        let auth_snapshot = TestSnapshot { id: 1, time: 2, sum: 100, items: vec![99] };
        history.apply_snapshot(auth_snapshot.clone());

        // Check that snapshot is stored as checkpoint
        let snap_key = last_key_at_time(2);
        assert_eq!(history.checkpoints.get(&snap_key), Some(&auth_snapshot));

        // Query at time 6 — should reflect snapshot + events at t=3, t=5
        let (result, _rx) = history.snapshot_at(6);
        // Base was overridden at t=2 with sum=100, then +20 at t=3, +30 at t=5 = 150
        assert_eq!(result.sum, 150);
        assert_eq!(result.time, 6);
    }

    #[test]
    fn test_apply_snapshot_then_query() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);

        let auth_snapshot = TestSnapshot { id: 1, time: 5, sum: 42, items: vec![1] };
        history.apply_snapshot(auth_snapshot);

        let (result, _rx) = history.snapshot_at(6);
        assert_eq!(result.sum, 42);
        assert_eq!(result.time, 6);
    }

    // --- advance tests ---

    #[test]
    fn test_advance_drops_old_events() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 100, 50);
        // lower_time_horizon_delta=50, current_time starts at 100
        // drop_time = current_time - 50

        // Add events at times 40, 60, 80
        history.apply_event(TestEvent::Positive(1, 40, 40, 1));
        history.apply_event(TestEvent::Positive(1, 60, 60, 2));
        history.apply_event(TestEvent::Positive(1, 80, 80, 3));

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
        history.apply_event(TestEvent::Positive(1, 10, 10, 5));
        history.apply_event(TestEvent::Positive(1, 20, 20, 10));
        history.apply_event(TestEvent::Positive(1, 30, 30, 15));

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

    // --- reconciliation tests ---

    #[test]
    fn test_in_order_event_no_reconciliation() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);

        history.apply_event(TestEvent::Positive(1, 1, 1, 10));
        let (_snapshot, reconciliation_rx) = history.snapshot_at(2);
        history.apply_event(TestEvent::Positive(1, 3, 3, 20));

        // No reconciliation should be sent for in-order events
        assert!(reconciliation_rx.try_recv().is_err());
    }

    #[test]
    fn test_out_of_order_event_sends_reconciliation() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);

        history.apply_event(TestEvent::Positive(1, 10, 10, 10));
        let (_snapshot, reconciliation_rx) = history.snapshot_at(11);
        history.apply_event(TestEvent::Positive(1, 5, 5, 20)); // out-of-order

        let recon = reconciliation_rx.try_recv().unwrap();
        assert_eq!(recon.snapshot_id, 1);
        assert_eq!(recon.from_time, 5);
        assert_eq!(recon.to_time, 10);
    }

    #[test]
    fn test_apply_snapshot_sends_reconciliation() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);

        history.apply_event(TestEvent::Positive(1, 10, 10, 10));
        let (_snapshot, reconciliation_rx) = history.snapshot_at(11);

        let auth_snapshot = TestSnapshot { id: 1, time: 5, sum: 42, items: vec![] };
        history.apply_snapshot(auth_snapshot);

        let recon = reconciliation_rx.try_recv().unwrap();
        assert_eq!(recon.snapshot_id, 1);
        assert_eq!(recon.from_time, 5);
        assert_eq!(recon.to_time, 10);
    }

    #[test]
    fn test_reconciliation_channel_full_no_panic() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);

        // Add an in-order event first
        history.apply_event(TestEvent::Positive(1, 100, 100, 1));
        let (_snapshot, _reconciliation_rx) = history.snapshot_at(101);

        // Fill the reconciliation subscriber channel (capacity 1000)
        for i in 0..1001 {
            // Each out-of-order event tries to send a reconciliation
            history.apply_event(TestEvent::Positive(1, i, i as u128, 1));
        }
        // Should not panic even when channel is full
    }
}
