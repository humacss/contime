//! Adapt event-owned policy checkpoint storage to managed worker messages.
use crate::contime_runtime::checkpoints::{ApplyEvents, ApplyWrapper, Snapshot};
use crate::contime_runtime::{RejectionMessage, SharedEvent};
use crate::contime_worker::{EventInsert, SnapshotStore};

pub(crate) struct EventPolicyCheckpoint<I, S, W>
where
    I: crate::contime_runtime::ThreadInput,
    S: Snapshot<Time = I::Time>,
{
    snapshot_id: u128,
    inner: crate::contime_core::EventPolicyStorage<I, S, W>,
}
impl<I, S, W> SnapshotStore<SharedEvent<I>> for EventPolicyCheckpoint<I, S, W>
where
    I: crate::contime_runtime::ThreadInput,
    S: Snapshot<Time = I::Time> + ApplyEvents<I>,
    W: ApplyWrapper<S, I>,
{
    type Config = crate::contime_core::CheckpointStorageConfig;
    type Context = W;
    type Time = I::Time;
    type Snapshot = S;
    type Rejection = RejectionMessage<crate::contime_runtime::RejectionReason>;
    fn create(id: u128, config: &Self::Config, horizon: &I::Time) -> Self {
        Self { snapshot_id: id, inner: crate::contime_core::EventPolicyStorage::create(id, config, horizon) }
    }
    fn event_time(input: &SharedEvent<I>) -> I::Time {
        input.inner.time()
    }
    fn affected_time(&self, time: &I::Time) -> I::Time {
        self.inner.affected_time(time)
    }
    fn earliest_replay_time(&self) -> Option<I::Time> {
        self.inner.earliest_replay_time()
    }
    fn insert(&mut self, input: SharedEvent<I>, horizon: &I::Time) -> EventInsert<Self::Rejection> {
        let input_time = input.inner.time();
        let result = self.inner.insert(input.inner, horizon);
        if let Some(observation) = input.observation {
            let minimum = result.changed.then(|| {
                let affected = self.inner.affected_time(&input_time);
                self.inner.earliest_replay_time().map_or(affected.clone(), |earliest| earliest.min(affected))
            });
            observation.lock().expect("admission observation poisoned").cover(self.snapshot_id, minimum);
        }
        EventInsert {
            changed: result.changed,
            rejections: result
                .rejections
                .into_iter()
                .map(|rejection| RejectionMessage { event_id: rejection.event_id, reason: rejection.reason })
                .collect(),
        }
    }
    fn process_until(&mut self, time: &I::Time, context: &mut W) {
        self.inner.process_until(time, context);
    }
    #[cfg(feature = "monitor")]
    fn process_until_counted(&mut self, time: &I::Time, context: &mut W) -> Option<u64> {
        self.inner.process_until_counted(time, context)
    }
    fn query(&mut self, time: I::Time, context: &mut W) -> Option<Box<S>> {
        self.inner.query(time, context)
    }
    fn query_events(&self, from: &I::Time, to: &I::Time) -> Vec<SharedEvent<I>> {
        self.inner.query_events(from, to).into_iter().map(|inner| SharedEvent { inner, observation: None }).collect()
    }
    fn forward(&mut self, time: &I::Time, context: &mut W) {
        self.inner.forward(time, context);
    }
    fn prune(&mut self) {
        self.inner.prune();
    }
}
