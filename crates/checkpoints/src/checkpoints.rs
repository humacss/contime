use std::collections::VecDeque;

use crate::types::ReplaySession;
use crate::{ApplyResult, Checkpoint, CheckpointConfig, CheckpointKey, CheckpointStore, Snapshot};

impl<S> CheckpointStore<S>
where
    S: Snapshot,
{
    /// Creates an empty checkpoint store for one snapshot ID.
    pub fn new(snapshot_id: u128, config: CheckpointConfig) -> Self {
        Self { snapshot_id, interval: config.interval, anchor: None, checkpoints: VecDeque::new(), retained_horizon: None }
    }

    pub fn snapshot_id(&self) -> u128 {
        self.snapshot_id
    }

    pub fn interval(&self) -> u64 {
        self.interval
    }

    pub fn len(&self) -> usize {
        self.checkpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.checkpoints.is_empty()
    }

    pub fn current(&self) -> Option<&Checkpoint<S>> {
        self.checkpoints.back()
    }

    pub fn anchor(&self) -> Option<&crate::ReplayAnchor<S>> {
        self.anchor.as_ref()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Checkpoint<S>> {
        self.checkpoints.iter()
    }

    /// Discards materialized state affected by a newly admitted event.
    /// Call before querying or resuming replay after canonical history changes.
    /// The timestamp's entire bucket must be reapplied, including earlier IDs.
    pub fn invalidate_from(&mut self, time: &S::Time) {
        let retained = self.checkpoints.partition_point(|checkpoint| checkpoint.key.time < *time);
        self.checkpoints.truncate(retained);
    }

    /// Earliest canonical bucket not represented by the current valid tip.
    pub fn next_replay_time<H: crate::Events<Time = S::Time>>(&self, events: &H) -> Option<S::Time> {
        events.iter_after(self.replay_boundary()).next().map(|event| event.time.clone())
    }

    pub(crate) fn replay_boundary(&self) -> Option<&CheckpointKey<S::Time>> {
        self.current().map(|checkpoint| &checkpoint.key).or_else(|| self.anchor.as_ref().and_then(|anchor| anchor.boundary.as_ref()))
    }

    /// Moves an incomplete tip into the next step instead of cloning it on
    /// every yield. Only fixed cadence checkpoints need a working-state clone.
    pub(crate) fn resume_replay(&mut self) -> ReplaySession<'_, S> {
        let movable_tip = self.checkpoints.len().checked_sub(1).filter(|index| !self.tip_interval_is_full(*index));
        let events_since_checkpoint = movable_tip.map_or(0, |index| self.events_in_checkpoint(index));
        let (working_snapshot, start_key, history_event_count) = if movable_tip.is_some() {
            let tip = self.checkpoints.pop_back().expect("movable tip exists");
            (Some(tip.snapshot), Some(tip.key), tip.history_event_count)
        } else if let Some(tip) = self.current() {
            (Some(tip.snapshot.clone()), Some(tip.key.clone()), tip.history_event_count)
        } else {
            self.anchor
                .as_ref()
                .map_or((None, None, 0), |anchor| (Some(anchor.snapshot.clone()), anchor.boundary.clone(), anchor.history_event_count))
        };
        let next_checkpoint_index = self.checkpoints.len();
        ReplaySession {
            store: self,
            working_snapshot,
            start_key,
            history_event_count,
            applied_events: 0,
            events_since_checkpoint,
            next_checkpoint_index,
        }
    }

    pub(crate) fn latest_before_index(&self, boundary: &CheckpointKey<S::Time>) -> Option<usize> {
        let index = self.checkpoints.partition_point(|checkpoint| checkpoint.key < *boundary);
        index.checked_sub(1)
    }

    pub(crate) fn begin_replay(&mut self, dirty_time: &S::Time) -> ReplaySession<'_, S> {
        let dirty_boundary = CheckpointKey { time: dirty_time.clone(), event_id: u128::MIN };
        let base_index = self.latest_before_index(&dirty_boundary);
        let (working_snapshot, start_key, history_event_count) = base_index.map_or_else(
            || {
                self.anchor
                    .as_ref()
                    .map_or((None, None, 0), |anchor| (Some(anchor.snapshot.clone()), anchor.boundary.clone(), anchor.history_event_count))
            },
            |index| {
                let checkpoint = &self.checkpoints[index];
                (Some(checkpoint.snapshot.clone()), Some(checkpoint.key.clone()), checkpoint.history_event_count)
            },
        );
        let movable_tip = base_index.filter(|index| *index + 1 == self.checkpoints.len() && !self.tip_interval_is_full(*index));
        let next_checkpoint_index = movable_tip.or_else(|| base_index.map(|index| index + 1)).unwrap_or(0);
        let events_since_checkpoint = movable_tip.map_or(0, |index| self.events_in_checkpoint(index));

        ReplaySession {
            store: self,
            working_snapshot,
            start_key,
            history_event_count,
            applied_events: 0,
            events_since_checkpoint,
            next_checkpoint_index,
        }
    }

    fn tip_interval_is_full(&self, tip_index: usize) -> bool {
        if self.interval == 0 {
            return false;
        }

        self.events_in_checkpoint(tip_index) >= self.interval
    }

    fn events_in_checkpoint(&self, index: usize) -> u64 {
        let previous_count = index.checked_sub(1).map_or_else(
            || self.anchor.as_ref().map_or(0, |anchor| anchor.history_event_count),
            |index| self.checkpoints[index].history_event_count,
        );
        self.checkpoints[index].history_event_count.saturating_sub(previous_count)
    }
}

impl<'a, S> ReplaySession<'a, S>
where
    S: Snapshot,
{
    pub(crate) fn start_key(&self) -> Option<&CheckpointKey<S::Time>> {
        self.start_key.as_ref()
    }

    pub(crate) fn initialize(&mut self, snapshot: S) {
        debug_assert!(self.working_snapshot.is_none());
        if self.store.anchor.is_none() {
            self.store.anchor = Some(crate::ReplayAnchor { boundary: None, snapshot: snapshot.clone(), history_event_count: 0 });
        }
        self.working_snapshot = Some(snapshot);
    }

    pub(crate) fn snapshot_mut(&mut self) -> &mut S {
        self.working_snapshot.as_mut().expect("replay snapshot must be initialized")
    }

    pub(crate) fn advance_event_count(&mut self, event_count: u64) -> u64 {
        self.history_event_count = self.history_event_count.checked_add(event_count).expect("history event count overflow");
        self.applied_events = self.applied_events.checked_add(event_count).expect("applied event count overflow");
        self.events_since_checkpoint = self.events_since_checkpoint.checked_add(event_count).expect("checkpoint event count overflow");
        self.history_event_count
    }

    pub(crate) fn record_intermediate(&mut self, key: CheckpointKey<S::Time>) {
        if self.store.interval == 0 || self.events_since_checkpoint < self.store.interval {
            return;
        }

        self.store_intermediate(key);
        self.next_checkpoint_index += 1;
        self.events_since_checkpoint = 0;
    }

    pub(crate) fn finish(mut self, key: CheckpointKey<S::Time>) -> ApplyResult {
        let snapshot = self.working_snapshot.take().expect("replay snapshot must be initialized");
        if self.next_checkpoint_index < self.store.checkpoints.len() {
            self.move_working_into_checkpoint(self.next_checkpoint_index, key, snapshot);
        } else {
            debug_assert_eq!(self.next_checkpoint_index, self.store.checkpoints.len());
            self.append_moved_checkpoint(key, snapshot);
        }
        self.store.checkpoints.truncate(self.next_checkpoint_index + 1);

        ApplyResult { applied_events: self.applied_events, retained_checkpoints: self.store.len() }
    }

    pub(crate) fn unchanged(self) -> ApplyResult {
        ApplyResult { applied_events: 0, retained_checkpoints: self.store.len() }
    }

    fn store_intermediate(&mut self, key: CheckpointKey<S::Time>) {
        let snapshot = self.working_snapshot.as_ref().expect("replay snapshot must be initialized");
        if self.next_checkpoint_index < self.store.checkpoints.len() {
            let checkpoint = &mut self.store.checkpoints[self.next_checkpoint_index];
            checkpoint.snapshot.clone_from(snapshot);
            checkpoint.key = key;
            checkpoint.history_event_count = self.history_event_count;
        } else {
            debug_assert_eq!(self.next_checkpoint_index, self.store.checkpoints.len());
            let snapshot = snapshot.clone();
            self.append_moved_checkpoint(key, snapshot);
        }
    }

    fn move_working_into_checkpoint(&mut self, index: usize, key: CheckpointKey<S::Time>, snapshot: S) {
        self.store.checkpoints[index] = Checkpoint { key, snapshot, history_event_count: self.history_event_count };
    }

    fn append_moved_checkpoint(&mut self, key: CheckpointKey<S::Time>, snapshot: S) {
        self.store.checkpoints.push_back(Checkpoint { key, snapshot, history_event_count: self.history_event_count });
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::{BatchSize, Criterion};

    use crate::{CheckpointConfig, CheckpointKey, CheckpointStore, Snapshot};

    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: i64,
        value: u64,
    }

    impl Snapshot for TestSnapshot {
        type Time = i64;

        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    fn key(value: u128) -> CheckpointKey<i64> {
        CheckpointKey { time: value as i64, event_id: value }
    }

    fn finish_event(store: &mut CheckpointStore<TestSnapshot>, value: u64) {
        let mut session = store.begin_replay(&(value as i64));
        if session.working_snapshot.is_none() {
            session.initialize(TestSnapshot::default());
        }
        session.advance_event_count(1);
        session.snapshot_mut().value = value;
        session.snapshot_mut().set_time(value as i64);
        session.finish(key(value as u128));
    }

    #[test]
    fn a_partial_tip_moves_in_place_and_a_full_tip_causes_an_append() {
        let mut store = CheckpointStore::new(7, CheckpointConfig { interval: 3 });

        finish_event(&mut store, 0);
        finish_event(&mut store, 1);
        finish_event(&mut store, 2);

        assert_eq!(store.len(), 1);
        assert_eq!(store.current().unwrap().key, key(2));
        assert_eq!(store.current().unwrap().history_event_count, 3);

        finish_event(&mut store, 3);

        assert_eq!(store.len(), 2);
        assert_eq!(store.iter().map(|checkpoint| checkpoint.key.clone()).collect::<Vec<_>>(), vec![key(2), key(3)]);
    }

    #[test]
    fn late_replay_updates_existing_checkpoint_slots_without_middle_insertion() {
        let mut store = CheckpointStore::new(7, CheckpointConfig { interval: 3 });
        for value in 0..4 {
            finish_event(&mut store, value);
        }
        let retained_count = store.len();

        let mut session = store.begin_replay(&0);
        if session.working_snapshot.is_none() {
            session.initialize(TestSnapshot::default());
        }
        session.advance_event_count(3);
        session.snapshot_mut().value = 30;
        session.record_intermediate(key(20));
        session.advance_event_count(2);
        session.snapshot_mut().value = 50;
        session.finish(key(40));

        assert_eq!(store.len(), retained_count);
        assert_eq!(store.iter().map(|checkpoint| checkpoint.key.clone()).collect::<Vec<_>>(), vec![key(20), key(40)]);
        assert_eq!(store.iter().map(|checkpoint| checkpoint.snapshot.value).collect::<Vec<_>>(), vec![30, 50]);
    }

    #[test]
    fn first_retained_tip_counts_only_events_after_the_anchor() {
        let mut store = CheckpointStore::new(7, CheckpointConfig { interval: 10 });
        store.anchor =
            Some(crate::ReplayAnchor { boundary: Some(key(6)), snapshot: TestSnapshot { time: 6, value: 6 }, history_event_count: 6 });
        store.checkpoints.push_back(crate::Checkpoint {
            key: key(10),
            snapshot: TestSnapshot { time: 10, value: 10 },
            history_event_count: 10,
        });
        assert_eq!(store.events_in_checkpoint(0), 4);
        assert!(!store.tip_interval_is_full(0));
        finish_event(&mut store, 11);
        assert_eq!(store.len(), 1);
        assert_eq!(store.current().unwrap().history_event_count, 11);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_checkpoints() {
        let mut criterion = Criterion::default();
        criterion.bench_function("checkpoints/store/1000_sequential_tip_updates", |bencher| {
            bencher.iter_batched(
                || CheckpointStore::new(7, CheckpointConfig { interval: 100 }),
                |mut store| {
                    for value in 0..1_000 {
                        finish_event(&mut store, value);
                    }
                    black_box(store)
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
