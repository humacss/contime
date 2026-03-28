use std::collections::BTreeMap;
use std::ops::Bound;

use flume::{bounded, Receiver, Sender};

use crate::{ApplyEvent, ContimeKey, Event, Snapshot};

use super::apply::replay_and_checkpoint;
use super::apply_event_in_place;

/// Dependency injection seam used by `SnapshotHistory` tests.
pub trait Deps: Default {
    fn apply_event_in_place<S: Snapshot>(
        &self,
        new_event: S::Event,
        base_snapshot: &S,
        checkpoints: &mut BTreeMap<ContimeKey, S>,
        events: &mut BTreeMap<ContimeKey, S::Event>,
        checkpoint_interval: usize,
    ) -> i64;
}

/// Default dependency set for [`SnapshotHistory`].
#[derive(Default)]
pub struct DefDeps;

impl Deps for DefDeps {
    fn apply_event_in_place<S: Snapshot>(
        &self,
        new_event: S::Event,
        base_snapshot: &S,
        checkpoints: &mut BTreeMap<ContimeKey, S>,
        events: &mut BTreeMap<ContimeKey, S::Event>,
        checkpoint_interval: usize,
    ) -> i64 {
        apply_event_in_place(new_event, base_snapshot, checkpoints, events, checkpoint_interval)
    }
}

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

/// Advanced per-snapshot history store used internally by `Contime`.
///
/// Most users should interact with [`crate::Contime`] instead. This type is useful when you
/// want direct control over one snapshot timeline, for example in benchmarks or focused tests.
#[derive(Debug, Clone)]
pub struct LocalSnapshotHistory<S, D>
where
    S: Snapshot,
    D: Deps,
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

    reconciliation_tx: Sender<Reconciliation>,
    reconciliation_rx: Receiver<Reconciliation>,

    deps: D,
}

const CHECKPOINT_INTERVAL: usize = 100;

impl<S, D> LocalSnapshotHistory<S, D>
where
    S: Snapshot + 'static,
    D: Deps,
{
    /// Creates a history for one snapshot and returns it with its initial memory cost.
    pub fn new(snapshot: S, current_time: i64, lower_time_horizon_delta: i64) -> (Self, i64) {
        let checkpoints = BTreeMap::new();
        let events = BTreeMap::new();
        let (reconciliation_tx, reconciliation_rx) = bounded(1000);

        let mut base_snapshot = snapshot.clone();
        base_snapshot.set_time(0);

        let base_size = base_snapshot.conservative_size() as i64;

        (
            Self {
                current_time,
                lower_time_horizon_delta,
                reconciliation_tx,
                reconciliation_rx,
                snapshot_id: snapshot.id(),
                base_snapshot,
                checkpoints,
                events,
                checkpoint_interval: CHECKPOINT_INTERVAL,
                deps: D::default(),
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
    pub fn apply_event(&mut self, event: S::Event) -> i64 {
        // Check if this event is out-of-order before applying
        let event_time = event.time();
        let latest_event_time = self.events.keys().next_back().map(|k| k.time);

        let bytes_delta =
            self.deps.apply_event_in_place(event, &self.base_snapshot, &mut self.checkpoints, &mut self.events, self.checkpoint_interval);

        // Send reconciliation if out-of-order (event is before the latest existing event)
        if let Some(latest_time) = latest_event_time {
            if event_time < latest_time {
                let _ = self.reconciliation_tx.try_send(Reconciliation {
                    snapshot_id: self.snapshot_id,
                    from_time: event_time,
                    to_time: latest_time,
                });
            }
        }

        bytes_delta
    }

    /// Applies an authoritative snapshot and replays later events on top of it.
    pub fn apply_snapshot(&mut self, snapshot: S) -> i64 {
        if snapshot.id() != self.snapshot_id {
            return 0;
        }

        let snapshot_key = ContimeKey { time: snapshot.time(), id: self.snapshot_id };

        let mut bytes_delta: i64 = 0;

        // Remove all checkpoints at or after snapshot_key
        let stale_keys: Vec<ContimeKey> = self.checkpoints.range(snapshot_key.clone()..).map(|(k, _)| k.clone()).collect();
        for key in &stale_keys {
            let removed = self.checkpoints.remove(key).expect("stale checkpoint key must exist");
            bytes_delta -= removed.conservative_size() as i64;
        }

        // Insert the snapshot as a checkpoint
        bytes_delta += snapshot.conservative_size() as i64;
        self.checkpoints.insert(snapshot_key.clone(), snapshot.clone());

        // Replay events after snapshot_key to rebuild downstream checkpoints
        bytes_delta +=
            replay_and_checkpoint(&snapshot, Bound::Excluded(&snapshot_key), &mut self.checkpoints, &self.events, self.checkpoint_interval);

        // Send reconciliation notification
        let latest_event_time = self.events.keys().next_back().map(|k| k.time).unwrap_or(snapshot.time());
        let _ = self.reconciliation_tx.try_send(Reconciliation {
            snapshot_id: self.snapshot_id,
            from_time: snapshot.time(),
            to_time: latest_event_time,
        });

        bytes_delta
    }

    /// Reconstructs the snapshot state at `time` and returns a reconciliation receiver.
    ///
    /// Events at exactly `time` are not included in the returned snapshot.
    pub fn snapshot_at(&self, time: i64) -> (S, Receiver<Reconciliation>) {
        let query_key = ContimeKey { time, id: self.snapshot_id };

        // Find the latest checkpoint at or before the query time
        let checkpoint_entry = self.checkpoints.range(..=&query_key).next_back();

        let (mut snapshot, replay_start) = match checkpoint_entry {
            Some((key, checkpoint)) => (checkpoint.clone(), Bound::Excluded(key.clone())),
            None => (self.base_snapshot.clone(), Bound::Unbounded),
        };

        let end_key = ContimeKey { time, id: self.snapshot_id };

        // Replay events from after the checkpoint up to the query time
        for (_key, event) in self.events.range((replay_start, Bound::Excluded(end_key))) {
            event.apply_to(&mut snapshot);
        }

        snapshot.set_time(time);

        (snapshot, self.reconciliation_rx.clone())
    }
}

/// Concrete per-snapshot history type used by the crate.
pub type SnapshotHistory<S> = LocalSnapshotHistory<S, DefDeps>;

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    use crate::{Event, TestEvent, TestSnapshot};

    #[rstest]
    #[case::empty(1, 2, (2, 3), vec![], vec![], 0, 1)]
    #[case::different_snapshot(1, 2, (3, 3), vec![], vec![], 0, 1)]
    fn test_apply_event(
        #[case] bytes_delta: i64,
        #[case] snapshot_id: u128,
        #[case] event: (u128, u128),
        #[case] checkpoints: Vec<i64>,
        #[case] events: Vec<(u128, i64)>,
        #[case] checkpoint_interval: usize,
        #[case] expected: i64,
    ) {
        struct TestDeps {
            new_event: TestEvent,
            snapshot_id: SnapshotId,
            checkpoints: BTreeMap<ContimeKey, TestSnapshot>,
            events: BTreeMap<ContimeKey, TestEvent>,
            checkpoint_interval: usize,
            bytes_delta: i64,
        }

        impl Default for TestDeps {
            fn default() -> Self {
                Self {
                    new_event: TestEvent::Positive(0, 0, 0, 0),
                    snapshot_id: 0,
                    checkpoints: BTreeMap::new(),
                    events: BTreeMap::new(),
                    checkpoint_interval: 0,
                    bytes_delta: 0,
                }
            }
        }

        impl Deps for TestDeps {
            fn apply_event_in_place<S: Snapshot>(
                &self,
                new_event: S::Event,
                _base_snapshot: &S,
                checkpoints: &mut BTreeMap<ContimeKey, S>,
                events: &mut BTreeMap<ContimeKey, S::Event>,
                checkpoint_interval: usize,
            ) -> i64 {
                assert_eq!(self.new_event.id(), new_event.id(), "wrong new_event input");

                assert_eq!(
                    self.events.keys().map(|k| k.id).collect::<Vec<u128>>(),
                    events.keys().map(|k| k.id).collect::<Vec<u128>>(),
                    "wrong events input"
                );

                assert_eq!(
                    self.checkpoints.keys().map(|k| (k.id, k.time)).collect::<Vec<(u128, i64)>>(),
                    checkpoints.keys().map(|k| (k.id, k.time)).collect::<Vec<(u128, i64)>>(),
                    "wrong checkpoints input"
                );

                assert_eq!(self.checkpoint_interval, checkpoint_interval, "wrong checkpoint interval input");

                self.bytes_delta
            }
        }

        let mut snapshot = TestSnapshot::default();
        snapshot.id = snapshot_id;

        let (mut history, _base_delta) = LocalSnapshotHistory::<TestSnapshot, TestDeps>::new(snapshot, 0, 1000);

        // Set up checkpoints
        history.checkpoints = checkpoints
            .into_iter()
            .map(|time| {
                let key = ContimeKey { time, id: snapshot_id };
                let snap = TestSnapshot { id: snapshot_id, time, items: vec![], sum: 0 };
                (key, snap)
            })
            .collect();

        // Set up events
        history.events = events
            .into_iter()
            .map(|(id, time)| {
                let key = ContimeKey { time, id };
                let event = TestEvent::Positive(id, time, snapshot_id, 1);
                (key, event)
            })
            .collect();

        history.checkpoint_interval = checkpoint_interval;

        let new_event = TestEvent::Positive(event.0, 0, event.1, 1);

        history.deps.new_event = new_event.clone();
        history.deps.snapshot_id = snapshot_id;
        history.deps.checkpoints = history.checkpoints.clone();
        history.deps.events = history.events.clone();
        history.deps.checkpoint_interval = history.checkpoint_interval;
        history.deps.bytes_delta = bytes_delta;

        let actual = history.apply_event(new_event.into());

        assert_eq!(expected, actual, "wrong bytes_delta output");
    }

    // --- apply_snapshot tests ---

    #[test]
    fn test_apply_snapshot_wrong_id() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);

        let wrong_snapshot = TestSnapshot { id: 2, time: 5, sum: 10, items: vec![1, 2] };
        let delta = history.apply_snapshot(wrong_snapshot);

        assert_eq!(delta, 0);
        assert!(history.checkpoints.is_empty());
    }

    #[test]
    fn test_apply_snapshot_no_events() {
        let snapshot = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(snapshot, 0, 1000);

        let auth_snapshot = TestSnapshot { id: 1, time: 5, sum: 10, items: vec![1, 2] };
        let expected_size = auth_snapshot.conservative_size() as i64;
        let delta = history.apply_snapshot(auth_snapshot.clone());

        assert_eq!(delta, expected_size);
        let key = ContimeKey { time: 5, id: 1 };
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
        let snap_key = ContimeKey { time: 2, id: 1 };
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

        let (result, _rx) = history.snapshot_at(5);
        assert_eq!(result.sum, 42);
        assert_eq!(result.time, 5);
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
        history.apply_event(TestEvent::Positive(1, 2, 2, 20));

        // No reconciliation should be sent for in-order events
        assert!(history.reconciliation_rx.try_recv().is_err());
    }

    #[test]
    fn test_out_of_order_event_sends_reconciliation() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);

        history.apply_event(TestEvent::Positive(1, 10, 10, 10));
        history.apply_event(TestEvent::Positive(1, 5, 5, 20)); // out-of-order

        let recon = history.reconciliation_rx.try_recv().unwrap();
        assert_eq!(recon.snapshot_id, 1);
        assert_eq!(recon.from_time, 5);
        assert_eq!(recon.to_time, 10);
    }

    #[test]
    fn test_apply_snapshot_sends_reconciliation() {
        let base = TestSnapshot { id: 1, time: 0, sum: 0, items: vec![] };
        let (mut history, _) = SnapshotHistory::new(base, 0, 1000);

        history.apply_event(TestEvent::Positive(1, 10, 10, 10));

        let auth_snapshot = TestSnapshot { id: 1, time: 5, sum: 42, items: vec![] };
        history.apply_snapshot(auth_snapshot);

        let recon = history.reconciliation_rx.try_recv().unwrap();
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

        // Fill the reconciliation channel (capacity 1000)
        for i in 0..1001 {
            // Each out-of-order event tries to send a reconciliation
            history.apply_event(TestEvent::Positive(1, i, i as u128, 1));
        }
        // Should not panic even when channel is full
    }
}
