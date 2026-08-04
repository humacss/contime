use std::marker::PhantomData;

use crate::{ApplyBatch, ApplyEvents, ContimeKey, Event, Snapshot};

use super::checkpoints::{
    apply_events_to_checkpoint, commit_applied_checkpoint, get_checkpoint_for_apply, AppliedCheckpoint, CheckpointForApply,
};
use super::LocalSnapshotHistory;

/// The public primitive for applying one same-timestamp event batch.
pub struct ApplyInner<S>
where
    S: Snapshot + ApplyEvents,
{
    _snapshot: PhantomData<S>,
}

impl<S> Copy for ApplyInner<S> where S: Snapshot + ApplyEvents {}

impl<S> Clone for ApplyInner<S>
where
    S: Snapshot + ApplyEvents,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Default for ApplyInner<S>
where
    S: Snapshot + ApplyEvents,
{
    fn default() -> Self {
        Self { _snapshot: PhantomData }
    }
}

impl<S> ApplyInner<S>
where
    S: Snapshot + ApplyEvents,
{
    pub fn apply_event_batch(&self, snapshot: &mut S, batch: ApplyBatch<'_, S::Event>) {
        let time = batch.time.clone();
        <S as ApplyEvents>::apply_events(snapshot, batch);
        snapshot.set_time(time);
    }
}

/// Decision returned by an apply wrapper after observing one same-timestamp
/// batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyDecision {
    /// Continue applying later batches in this replay/apply pass.
    Continue,
    /// Exit the current replay/apply pass early because the wrapper knows
    /// continuing would not produce new state.
    EarlyExit,
}

/// Neutral, infallible extension seam around normal same-timestamp batch application.
///
/// Return [`ApplyDecision::EarlyExit`] to stop the current replay pass intentionally.
/// A panic indicates a broken invariant, and the caller must not assume the affected
/// `contime` instance remains usable afterward.
pub trait ApplyWrapper<S>
where
    S: Snapshot + ApplyEvents,
{
    fn apply_event_batch_wrapper(&mut self, snapshot: &mut S, batch: ApplyBatch<'_, S::Event>, apply_inner: ApplyInner<S>)
        -> ApplyDecision;
}

impl<S> ApplyWrapper<S> for ()
where
    S: Snapshot + ApplyEvents,
{
    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut S,
        batch: ApplyBatch<'_, S::Event>,
        apply_inner: ApplyInner<S>,
    ) -> ApplyDecision {
        apply_inner.apply_event_batch(snapshot, batch);
        ApplyDecision::Continue
    }
}

impl<S> LocalSnapshotHistory<S>
where
    S: Snapshot + ApplyEvents + 'static,
{
    /// Applies one incoming event batch to this history and returns the memory delta.
    pub fn apply_event_batch<C>(&mut self, events: Vec<S::Event>, context: &mut C) -> i64
    where
        C: ApplyWrapper<S>,
    {
        let applied_batch = self.insert_event_batch(events);
        if !applied_batch.changed {
            return 0;
        }

        let checkpoint = self.get_checkpoint_for_apply(
            applied_batch.earliest_changed_time,
            applied_batch.latest_event_key_before_apply,
            applied_batch.single_changed_event_key,
            applied_batch.changed_event_count,
        );
        let applied_checkpoint = self.apply_events_to_checkpoint(checkpoint, context);
        let bytes_delta = applied_batch.bytes_delta + self.commit_applied_checkpoint(applied_checkpoint);

        bytes_delta
    }

    fn get_checkpoint_for_apply(
        &mut self,
        earliest_changed_time: S::Time,
        latest_event_key_before_apply: Option<ContimeKey<S::Time>>,
        single_changed_event_key: Option<ContimeKey<S::Time>>,
        changed_event_count: usize,
    ) -> CheckpointForApply<S> {
        get_checkpoint_for_apply(self, earliest_changed_time, latest_event_key_before_apply, single_changed_event_key, changed_event_count)
    }

    fn apply_events_to_checkpoint<C>(&self, checkpoint: CheckpointForApply<S>, context: &mut C) -> AppliedCheckpoint<S>
    where
        C: ApplyWrapper<S>,
    {
        apply_events_to_checkpoint(self, checkpoint, context)
    }

    fn commit_applied_checkpoint(&mut self, applied_checkpoint: AppliedCheckpoint<S>) -> i64 {
        commit_applied_checkpoint(self, applied_checkpoint)
    }

    fn insert_event_batch(&mut self, events: Vec<S::Event>) -> InsertedEventBatch<S::Time> {
        let latest_event_key_before_apply = self.events.keys().next_back().cloned();
        let mut earliest_changed_time: Option<S::Time> = None;
        let mut bytes_delta = 0;
        let mut changed_event_count = 0usize;
        let mut single_changed_event_key = None;

        for event in events {
            let event_time = event.time();
            let event_key = ContimeKey::from_event(&event);
            earliest_changed_time = Some(match earliest_changed_time {
                Some(current) => current.min(event_time),
                None => event_time,
            });

            if self.events.get(&event_key).is_some_and(|existing| existing == &event) {
                continue;
            }

            if let Some(existing) = self.events.insert(event_key.clone(), event) {
                bytes_delta -= existing.conservative_size() as i64;
            }

            bytes_delta += self.events.get(&event_key).map(|event| event.conservative_size() as i64).unwrap_or_default();
            changed_event_count += 1;
            single_changed_event_key = (changed_event_count == 1).then_some(event_key);
        }

        InsertedEventBatch {
            changed: changed_event_count != 0,
            bytes_delta,
            earliest_changed_time: earliest_changed_time.unwrap_or_default(),
            latest_event_key_before_apply,
            single_changed_event_key,
            changed_event_count,
        }
    }
}

struct InsertedEventBatch<T: crate::ContimeTime> {
    changed: bool,
    bytes_delta: i64,
    earliest_changed_time: T,
    latest_event_key_before_apply: Option<ContimeKey<T>>,
    single_changed_event_key: Option<ContimeKey<T>>,
    changed_event_count: usize,
}
