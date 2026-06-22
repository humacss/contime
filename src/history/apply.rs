use std::convert::Infallible;
use std::marker::PhantomData;

use crate::{ApplyBatch, ApplyEvents, ContimeKey, Event, Snapshot};

use super::checkpoints::update_checkpoints_after_event_batch;
use super::LocalSnapshotHistory;

/// Error returned by an [`ApplyWrapper`] while applying one event batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    message: String,
}

impl ApplyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApplyError {}

impl From<Infallible> for ApplyError {
    fn from(error: Infallible) -> Self {
        match error {}
    }
}

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
    type Error: Into<ApplyError>;

    fn extra_batches(&mut self, _batch: ApplyBatch<'_, S::Event>) -> Result<Vec<Vec<S::Event>>, Self::Error> {
        Ok(Vec::new())
    }

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

    fn extra_batches(&mut self, _batch: ApplyBatch<'_, S::Event>) -> Result<Vec<Vec<S::Event>>, Self::Error> {
        Ok(Vec::new())
    }

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
    pub fn apply_event_batch<C>(&mut self, events: Vec<S::Event>, context: &mut C) -> Result<i64, ApplyError>
    where
        C: ApplyWrapper<S>,
    {
        let applied_batch = self.insert_event_batch(events);
        if !applied_batch.changed {
            return Ok(0);
        }

        let mut bytes_delta = applied_batch.bytes_delta;
        bytes_delta += update_checkpoints_after_event_batch(
            self,
            applied_batch.earliest_changed_time,
            applied_batch.latest_changed_time,
            applied_batch.latest_event_key_before_apply,
            applied_batch.single_changed_event_key,
            context,
        )?;

        Ok(bytes_delta)
    }

    fn insert_event_batch(&mut self, events: Vec<S::Event>) -> InsertedEventBatch {
        let latest_event_key_before_apply = self.events.keys().next_back().cloned();
        let mut earliest_changed_time = i64::MAX;
        let mut latest_changed_time = i64::MIN;
        let mut bytes_delta = 0;
        let mut changed_event_count = 0usize;
        let mut single_changed_event_key = None;

        for event in events {
            let event_time = event.time();
            let event_key = ContimeKey::from_event(&event);
            earliest_changed_time = earliest_changed_time.min(event_time);
            latest_changed_time = latest_changed_time.max(event_time);

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
            latest_changed_time,
            latest_event_key_before_apply,
            single_changed_event_key,
        }
    }
}

struct InsertedEventBatch {
    changed: bool,
    bytes_delta: i64,
    earliest_changed_time: i64,
    latest_changed_time: i64,
    latest_event_key_before_apply: Option<ContimeKey>,
    single_changed_event_key: Option<ContimeKey>,
}
