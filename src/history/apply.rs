use std::collections::btree_map::Entry;

use crate::{ContimeKey, Input, InputBatch, InputLanes, Snapshot, SnapshotLanes};

use super::checkpoints::{
    apply_events_to_checkpoint, commit_applied_checkpoint, get_checkpoint_for_apply, AppliedCheckpoint, CheckpointForApply,
};
use super::LocalSnapshotHistory;

/// The public primitive for applying effective event batches within one raw history bucket.
pub struct ApplyInner<'a, S>
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
    snapshot: &'a mut S,
    history_input_count: u64,
    apply_count: usize,
}

impl<'a, S> ApplyInner<'a, S>
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
    /// Creates an isolated apply operation over one mutable snapshot.
    pub fn new(snapshot: &'a mut S, history_input_count: u64) -> Self {
        Self { snapshot, history_input_count, apply_count: 0 }
    }

    /// Returns the cumulative raw input count represented by this history bucket.
    pub const fn history_input_count(&self) -> u64 {
        self.history_input_count
    }

    /// Applies one effective event batch and returns its raw history input count.
    ///
    /// Every effective partition of one raw history bucket receives the same
    /// cumulative count. An empty effective batch does not mutate the snapshot.
    pub fn apply_input_batch(&mut self, batch: InputBatch<'_, S::Input>) -> u64 {
        if !batch.inputs.is_empty() {
            let time = batch.time.clone();
            <S::Input as InputLanes<S>>::apply_events(self.snapshot, batch, self.history_input_count);
            self.snapshot.set_time(time);
        }

        self.apply_count += 1;
        self.history_input_count
    }

    /// Returns the snapshot after all inner applies completed so far.
    pub fn snapshot(&self) -> &S {
        self.snapshot
    }

    pub(crate) const fn has_applied(&self) -> bool {
        self.apply_count != 0
    }
}

/// Neutral, infallible extension seam around normal same-timestamp batch application.
///
/// A panic indicates a broken invariant, and the caller must not assume the affected
/// `contime` instance remains usable afterward.
///
/// Implementations must call `apply_inner` at least once. They may filter or
/// repartition events and inspect the resulting snapshot through
/// [`ApplyInner::snapshot`]. Mutable snapshot access remains encapsulated so
/// every available mutation is represented by an applied event batch.
pub trait ApplyWrapper<S>
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
    fn apply_input_batch_wrapper(&mut self, batch: InputBatch<'_, S::Input>, apply_inner: &mut ApplyInner<'_, S>) {
        apply_inner.apply_input_batch(batch);
    }

    /// Applies one canonical-history batch during input reconciliation.
    ///
    /// The default preserves ordinary application semantics. Integrations may
    /// override this method to observe authoritative history changes without
    /// repeating those effects during queries or retention reconstruction.
    fn reconcile_input_batch_wrapper(&mut self, batch: InputBatch<'_, S::Input>, apply_inner: &mut ApplyInner<'_, S>) {
        self.apply_input_batch_wrapper(batch, apply_inner);
    }
}

impl<S> ApplyWrapper<S> for ()
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
}

impl<S> LocalSnapshotHistory<S>
where
    S: SnapshotLanes + 'static,
    S::Input: InputLanes<S>,
{
    /// Applies one incoming temporal input batch and returns the memory delta.
    pub fn apply_input_batch<C>(&mut self, inputs: Vec<S::Input>, context: &mut C) -> i64
    where
        C: ApplyWrapper<S>,
    {
        let applied_batch = self.insert_input_batch(inputs);
        if !applied_batch.changed {
            return 0;
        }

        let Some(checkpoint) = self.get_checkpoint_for_apply(
            applied_batch.earliest_changed_time,
            applied_batch.latest_input_key_before_apply,
            applied_batch.single_changed_input_key,
            applied_batch.changed_event_count,
        ) else {
            let checkpoint_bytes = self
                .checkpoints
                .drain(..)
                .map(|(_key, checkpoint, _history_input_count)| checkpoint.conservative_size() as i64 + size_of::<u64>() as i64)
                .sum::<i64>();
            return applied_batch.bytes_delta - checkpoint_bytes;
        };
        let applied_checkpoint = self.apply_inputs_to_checkpoint(checkpoint, context);
        applied_batch.bytes_delta + self.commit_applied_checkpoint(applied_checkpoint)
    }

    fn get_checkpoint_for_apply(
        &mut self,
        earliest_changed_time: S::Time,
        latest_input_key_before_apply: Option<ContimeKey<S::Time>>,
        single_changed_input_key: Option<ContimeKey<S::Time>>,
        changed_event_count: usize,
    ) -> Option<CheckpointForApply<S>> {
        get_checkpoint_for_apply(self, earliest_changed_time, latest_input_key_before_apply, single_changed_input_key, changed_event_count)
    }

    fn apply_inputs_to_checkpoint<C>(&self, checkpoint: CheckpointForApply<S>, context: &mut C) -> AppliedCheckpoint<S>
    where
        C: ApplyWrapper<S>,
    {
        apply_events_to_checkpoint(self, checkpoint, context)
    }

    fn commit_applied_checkpoint(&mut self, applied_checkpoint: AppliedCheckpoint<S>) -> i64 {
        commit_applied_checkpoint(self, applied_checkpoint)
    }

    fn insert_input_batch(&mut self, inputs: Vec<S::Input>) -> InsertedInputBatch<S::Time> {
        let latest_input_key_before_apply = self.latest_input_key();
        let mut earliest_changed_time: Option<S::Time> = None;
        let mut bytes_delta = 0;
        let mut changed_event_count = 0usize;
        let mut single_changed_event_key = None;
        for input in inputs {
            let input_time = Input::time(&input);
            let input_key = ContimeKey::from_input(&input);
            earliest_changed_time = Some(match earliest_changed_time {
                Some(current) => current.min(input_time),
                None => input_time,
            });

            let Entry::Vacant(entry) = self.inputs.entry(input_key.clone()) else { continue };

            if input.is_event() {
                let input_lane_index = S::input_lane_index(self.snapshot_id, &input).unwrap_or_else(|| {
                    panic!("snapshot id {} received an event that cannot materialize its snapshot lane", self.snapshot_id)
                });
                match self.snapshot_lane_index {
                    Some(snapshot_lane_index) if snapshot_lane_index != input_lane_index => {
                        panic!("snapshot id {} received an event for a different snapshot lane", self.snapshot_id);
                    }
                    Some(_) => {}
                    None => self.snapshot_lane_index = Some(input_lane_index),
                }
            }

            bytes_delta += input.conservative_size() as i64;
            entry.insert(input);
            changed_event_count += 1;
            single_changed_event_key = (changed_event_count == 1).then_some(input_key);
        }

        InsertedInputBatch {
            changed: changed_event_count != 0,
            bytes_delta,
            earliest_changed_time: earliest_changed_time.unwrap_or_default(),
            latest_input_key_before_apply,
            single_changed_input_key: single_changed_event_key,
            changed_event_count,
        }
    }

    pub(super) fn latest_input_key(&self) -> Option<ContimeKey<S::Time>> {
        self.inputs.keys().next_back().cloned()
    }
}

struct InsertedInputBatch<T: crate::ContimeTime> {
    changed: bool,
    bytes_delta: i64,
    earliest_changed_time: T,
    latest_input_key_before_apply: Option<ContimeKey<T>>,
    single_changed_input_key: Option<ContimeKey<T>>,
    changed_event_count: usize,
}
