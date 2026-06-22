use std::convert::Infallible;
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
        <S as ApplyEvents>::apply_events(snapshot, batch);
        snapshot.set_time(batch.time);
    }
}

/// Neutral extension seam around normal same-timestamp batch application.
pub trait ApplyWrapper<S>
where
    S: Snapshot + ApplyEvents,
{
    type Error;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut S,
        batch: ApplyBatch<'_, S::Event>,
        apply_inner: ApplyInner<S>,
    ) -> Result<(), Self::Error>;
}

impl<S> ApplyWrapper<S> for ()
where
    S: Snapshot + ApplyEvents,
{
    type Error = Infallible;

    fn apply_event_batch_wrapper(
        &mut self,
        snapshot: &mut S,
        batch: ApplyBatch<'_, S::Event>,
        apply_inner: ApplyInner<S>,
    ) -> Result<(), Self::Error> {
        apply_inner.apply_event_batch(snapshot, batch);
        Ok(())
    }
}

impl<S> LocalSnapshotHistory<S>
where
    S: Snapshot + ApplyEvents + 'static,
{
    /// Applies one incoming event batch to this history and returns the memory delta.
    pub fn apply_event_batch<C>(&mut self, events: Vec<S::Event>, context: &mut C) -> Result<i64, C::Error>
    where
        C: ApplyWrapper<S>,
    {
        let applied_batch = self.insert_event_batch(events);
        if !applied_batch.changed {
            return Ok(0);
        }

        let checkpoint = self.get_checkpoint_for_apply(
            applied_batch.earliest_changed_time,
            applied_batch.latest_event_key_before_apply,
            applied_batch.single_changed_event_key,
        );
        let applied_checkpoint = self.apply_events_to_checkpoint(checkpoint, context)?;
        let bytes_delta = applied_batch.bytes_delta + self.commit_applied_checkpoint(applied_checkpoint);

        Ok(bytes_delta)
    }

    fn get_checkpoint_for_apply(
        &mut self,
        earliest_changed_time: i64,
        latest_event_key_before_apply: Option<ContimeKey>,
        single_changed_event_key: Option<ContimeKey>,
    ) -> CheckpointForApply<S> {
        get_checkpoint_for_apply(self, earliest_changed_time, latest_event_key_before_apply, single_changed_event_key)
    }

    fn apply_events_to_checkpoint<C>(&self, checkpoint: CheckpointForApply<S>, context: &mut C) -> Result<AppliedCheckpoint<S>, C::Error>
    where
        C: ApplyWrapper<S>,
    {
        apply_events_to_checkpoint(self, checkpoint, context)
    }

    fn commit_applied_checkpoint(&mut self, applied_checkpoint: AppliedCheckpoint<S>) -> i64 {
        commit_applied_checkpoint(self, applied_checkpoint)
    }

    fn insert_event_batch(&mut self, events: Vec<S::Event>) -> InsertedEventBatch {
        let latest_event_key_before_apply = self.events.keys().next_back().cloned();
        let mut earliest_changed_time = i64::MAX;
        let mut bytes_delta = 0;
        let mut changed_event_count = 0usize;
        let mut single_changed_event_key = None;

        for event in events {
            let event_time = event.time();
            let event_key = ContimeKey::from_event(&event);
            earliest_changed_time = earliest_changed_time.min(event_time);

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
            earliest_changed_time,
            latest_event_key_before_apply,
            single_changed_event_key,
        }
    }
}

struct InsertedEventBatch {
    changed: bool,
    bytes_delta: i64,
    earliest_changed_time: i64,
    latest_event_key_before_apply: Option<ContimeKey>,
    single_changed_event_key: Option<ContimeKey>,
}
