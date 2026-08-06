use std::marker::PhantomData;

use crate::{ContimeKey, Input, InputBatch, InputLanes, Snapshot, SnapshotLanes};

use super::checkpoints::{
    apply_events_to_checkpoint, commit_applied_checkpoint, get_checkpoint_for_apply, AppliedCheckpoint, CheckpointForApply,
};
use super::LocalSnapshotHistory;

/// The public primitive for applying the event subset of one same-time input batch.
pub struct ApplyInner<S>
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
    _snapshot: PhantomData<S>,
}

impl<S> Copy for ApplyInner<S>
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
}

impl<S> Clone for ApplyInner<S>
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Default for ApplyInner<S>
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
    fn default() -> Self {
        Self { _snapshot: PhantomData }
    }
}

impl<S> ApplyInner<S>
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
    pub fn apply_input_batch(&self, snapshot: &mut S, batch: InputBatch<'_, S::Input>) {
        if !batch.inputs.iter().any(|input| input.is_event()) {
            return;
        }

        let time = batch.time.clone();
        <S::Input as InputLanes<S>>::apply_events(snapshot, batch);
        snapshot.set_time(time);
    }
}

/// Neutral, infallible extension seam around normal same-timestamp batch application.
///
/// A panic indicates a broken invariant, and the caller must not assume the affected
/// `contime` instance remains usable afterward.
pub trait ApplyWrapper<S>
where
    S: Snapshot,
    S::Input: InputLanes<S>,
{
    fn apply_input_batch_wrapper(&mut self, snapshot: &mut S, batch: InputBatch<'_, S::Input>, apply_inner: ApplyInner<S>) {
        apply_inner.apply_input_batch(snapshot, batch);
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
            let checkpoint_bytes = self.checkpoints.drain(..).map(|(_key, checkpoint)| checkpoint.conservative_size() as i64).sum::<i64>();
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

            if self.inputs.get(&input_key).is_some_and(|existing| existing == &input) {
                continue;
            }

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

            if let Some(existing) = self.inputs.insert(input_key.clone(), input) {
                bytes_delta -= existing.conservative_size() as i64;
            }

            bytes_delta += self.inputs.get(&input_key).map(|input| input.conservative_size() as i64).unwrap_or_default();
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
