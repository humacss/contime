//! Native admission coverage around the shared synchronous storage adapter.
use crate::contime_runtime::checkpoints::{ApplyEvents, ApplyWrapper, Snapshot};
use crate::contime_runtime::types::{CheckpointStorage, CheckpointStorageConfig};
use crate::contime_runtime::{RejectionMessage, RejectionReason, SharedEvent};
use crate::contime_worker::{EventInsert, SnapshotStore};
impl<I, S, W> SnapshotStore<SharedEvent<I>> for CheckpointStorage<I, S, W>
where
    I: crate::contime_runtime::ThreadInput,
    S: Snapshot<Time = I::Time> + ApplyEvents<I>,
    W: ApplyWrapper<S, I>,
{
    type Config = CheckpointStorageConfig;
    type Context = W;
    type Time = I::Time;
    type Snapshot = S;
    type Rejection = RejectionMessage<RejectionReason>;
    fn create(snapshot_id: u128, config: &Self::Config, horizon: &I::Time) -> Self {
        Self { snapshot_id, inner: crate::contime_core::CheckpointStorage::create(snapshot_id, config, horizon) }
    }
    fn event_time(input: &SharedEvent<I>) -> I::Time {
        input.inner.time()
    }
    fn earliest_replay_time(&self) -> Option<I::Time> {
        self.inner.earliest_replay_time()
    }
    fn insert(&mut self, input: SharedEvent<I>, horizon: &I::Time) -> EventInsert<Self::Rejection> {
        let result = self.inner.insert(input.inner, horizon);
        if let Some(observation) = input.observation {
            let minimum = result.changed.then(|| self.inner.earliest_replay_time()).flatten();
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
    #[cfg(feature = "counters")]
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::contime_runtime::checkpoints::{ApplyBatch, ApplyInner, CheckpointConfig, EventBatch};
    use crate::contime_runtime::types::testing::Time;
    use crate::contime_runtime::Input;
    use crate::contime_worker::SnapshotStore;

    struct TestEvent {
        id: u128,
        time: Time,
        value: i64,
    }
    impl crate::contime_checkpoints::Event for TestEvent {
        type Time = Time;
        fn time(&self) -> Time {
            self.time
        }
    }
    impl Input for TestEvent {
        fn event_id(&self) -> u128 {
            self.id
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }
    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: Time,
        id: u128,
        compacted: i64,
        recent: i64,
        counts: Vec<u64>,
    }
    impl Snapshot for TestSnapshot {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.time
        }
        fn set_time(&mut self, time: Time) {
            self.time = time;
        }
    }
    impl ApplyEvents<TestEvent> for TestSnapshot {
        fn create(id: u128, _: &TestEvent) -> Self {
            Self { id, ..Self::default() }
        }
        fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, TestEvent>) {
            self.recent += batch.events.map(|event| event.value).sum::<i64>();
            self.counts.push(batch.history_event_count);
        }
    }
    #[derive(Default)]
    struct Context {
        effects: Vec<Time>,
        forwards: Vec<Time>,
        limit: Option<usize>,
    }
    impl ApplyWrapper<TestSnapshot, TestEvent> for Context {
        fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, Time, TestEvent>, inner: &mut ApplyInner<'_, TestSnapshot>) {
            self.effects.push(batch.time);
            let lookahead = batch.lookahead;
            let mut events = batch.events.take(self.limit.unwrap_or(usize::MAX));
            inner.apply_event_batch(EventBatch { snapshot_id: batch.snapshot_id, time: batch.time, events: &mut events, lookahead });
        }
        fn retain_snapshot(&mut self, snapshot: &mut TestSnapshot, time: &Time) {
            snapshot.compacted += std::mem::take(&mut snapshot.recent);
            self.forwards.push(*time);
        }
    }
    type Adapter = CheckpointStorage<TestEvent, TestSnapshot, Context>;
    fn event(id: u128, time: i64, value: i64) -> SharedEvent<TestEvent> {
        crate::contime_runtime::input::prepare_inputs(vec![TestEvent { id, time: Time(time), value }]).pop().unwrap()
    }

    #[derive(Default)]
    struct LookaheadContext(Vec<Option<Time>>);

    impl ApplyWrapper<TestSnapshot, TestEvent> for LookaheadContext {
        fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, Time, TestEvent>, inner: &mut ApplyInner<'_, TestSnapshot>) {
            let lookahead = batch.lookahead;
            inner.apply_event_batch(batch);
            self.0.push(lookahead.next_event_time());
        }
    }

    #[test]
    fn replay_wrapper_observes_the_next_canonical_timestamp() {
        type LookaheadAdapter = CheckpointStorage<TestEvent, TestSnapshot, LookaheadContext>;

        let mut store = LookaheadAdapter::create(7, &CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 1 } }, &Time(0));
        let mut context = LookaheadContext::default();
        store.insert(event(1, 10, 1), &Time(0));
        store.insert(event(2, 20, 2), &Time(0));

        store.process_until(&Time(20), &mut context);

        assert_eq!(context.0, vec![Some(Time(20)), None]);
    }

    #[test]
    fn happy() {
        let expected_sum = 7;
        let expected_counts = vec![0, 2];
        let expected_effects = vec![Time(10), Time(20)];

        let mut store = Adapter::create(7, &CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 1 } }, &Time(0));
        let mut context = Context::default();
        for event in [event(2, 10, 2), event(3, 20, 4), event(1, 10, 1)] {
            assert!(store.insert(event, &Time(0)).changed);
        }

        let queried = store.query(Time(20), &mut context).unwrap();
        let query_effects = context.effects.clone();
        store.process_until(&Time(20), &mut context);
        let actual = store.query(Time(20), &mut context).unwrap();

        assert_eq!(actual.id, 7);
        assert_eq!(actual.recent, expected_sum);
        assert_eq!(queried.recent, expected_sum);
        assert_eq!(actual.counts, expected_counts);
        assert!(query_effects.is_empty());
        assert_eq!(context.effects, expected_effects);
    }

    #[test]
    fn admission_coverage_waits_for_all_routes_and_is_not_retained() {
        let config = CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 100 } };
        let mut first = Adapter::create(7, &config, &Time(20));
        let mut second = Adapter::create(8, &config, &Time(20));
        let observation = std::sync::Arc::new(std::sync::Mutex::new(crate::contime_progress::CoverageConstraint::new(Time(20), [7, 8])));
        let mut input = event(1, 90, 1);
        input.observation = Some(observation.clone());

        assert!(first.insert(input.clone(), &Time(20)).changed);
        assert_eq!(observation.lock().unwrap().boundary(), Some(Time(20)));
        assert!(second.insert(input, &Time(20)).changed);
        assert_eq!(observation.lock().unwrap().boundary(), Some(Time(90)));
        assert_eq!(std::sync::Arc::strong_count(&observation), 1);
        assert!(first.query_events(&Time(20), &Time(100))[0].observation.is_none());
        assert!(second.query_events(&Time(20), &Time(100))[0].observation.is_none());
    }

    #[test]
    fn rejected_and_duplicate_admissions_cover_routes_without_adding_work() {
        let config = CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 100 } };
        let mut store = Adapter::create(7, &config, &Time(20));
        assert!(store.insert(event(1, 90, 1), &Time(20)).changed);
        for mut input in [event(1, 90, 1), event(2, 10, 1)] {
            let observation = std::sync::Arc::new(std::sync::Mutex::new(crate::contime_progress::CoverageConstraint::new(Time(20), [7])));
            input.observation = Some(observation.clone());
            assert!(!store.insert(input, &Time(20)).changed);
            assert_eq!(observation.lock().unwrap().boundary(), None);
            assert_eq!(std::sync::Arc::strong_count(&observation), 1);
        }
    }

    #[test]
    fn partially_consumed_batches_still_count_all_canonical_events() {
        let expected_sum = 5;
        let expected_counts = vec![0, 3];

        let mut store = Adapter::create(7, &CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 1 } }, &Time(0));
        let mut context = Context { limit: Some(1), ..Context::default() };
        for event in [event(1, 10, 1), event(2, 10, 2), event(3, 10, 3), event(4, 20, 4)] {
            store.insert(event, &Time(0));
        }

        store.process_until(&Time(20), &mut context);
        let actual = store.query(Time(20), &mut context).unwrap();

        assert_eq!(actual.recent, expected_sum);
        assert_eq!(actual.counts, expected_counts);
    }

    #[test]
    fn duplicate_does_not_replay_and_late_insertion_does() {
        let expected_sum = 7;
        let expected_counts = vec![0, 2];

        let mut store = Adapter::create(7, &CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 1 } }, &Time(0));
        let mut context = Context::default();
        store.insert(event(1, 10, 1), &Time(0));
        store.insert(event(2, 20, 2), &Time(0));
        store.process_until(&Time(20), &mut context);
        context.effects.clear();

        let duplicate = store.insert(event(1, 5, 999), &Time(0));
        store.process_until(&Time(20), &mut context);
        let duplicate_effects = context.effects.clone();
        store.insert(event(3, 10, 4), &Time(0));
        store.process_until(&Time(20), &mut context);
        let actual = store.query(Time(20), &mut context).unwrap();

        assert!(!duplicate.changed);
        assert!(duplicate_effects.is_empty());
        assert_eq!(actual.recent, expected_sum);
        assert_eq!(actual.counts, expected_counts);
        assert_eq!(context.effects, vec![Time(10), Time(20)]);
    }

    #[test]
    fn forwarding_compacts_state_and_pruning_preserves_boundary_events() {
        let expected_sum = 7;
        let expected_compacted = 3;
        let expected_hooks = vec![Time(10), Time(19)];
        let expected_effects = vec![Time(20)];

        let mut store = Adapter::create(7, &CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 1 } }, &Time(0));
        let mut context = Context::default();
        store.insert(event(1, 10, 3), &Time(0));
        store.insert(event(2, 20, 4), &Time(0));

        store.forward(&Time(20), &mut context);
        let forwarding_effects = context.effects.clone();
        store.prune();
        let rejected_query = store.query(Time(19), &mut context);
        let retained = store.query_events(&Time(0), &Time(30));
        store.process_until(&Time(20), &mut context);
        let actual = store.query(Time(20), &mut context).unwrap();

        assert!(rejected_query.is_none());
        assert!(forwarding_effects.is_empty());
        assert_eq!(retained.iter().map(|event| event.event_id()).collect::<Vec<_>>(), vec![2]);
        assert_eq!(actual.compacted, expected_compacted);
        assert_eq!(actual.compacted + actual.recent, expected_sum);
        assert_eq!(context.forwards, expected_hooks);
        assert_eq!(context.effects, expected_effects);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn counted_replay_reports_raw_events_despite_partial_wrapper_consumption() {
        let config = CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 3 } };
        let mut store = Adapter::create(7, &config, &Time(0));
        let mut context = Context { limit: Some(1), ..Context::default() };
        assert_eq!(store.process_until_counted(&Time(20), &mut context), Some(0));
        for input in [event(1, 10, 1), event(2, 10, 2), event(3, 20, 4)] {
            store.insert(input, &Time(0));
        }
        assert_eq!(store.process_until_counted(&Time(20), &mut context), Some(3));
        assert_eq!(store.process_until_counted(&Time(20), &mut context), Some(0));
        assert_eq!(context.effects, vec![Time(10), Time(20)]);
        assert!(!store.insert(event(1, 10, 1), &Time(0)).changed);
        assert_eq!(store.process_until_counted(&Time(20), &mut context), Some(0));

        store.insert(event(4, 15, 8), &Time(0));
        assert_eq!(store.process_until_counted(&Time(20), &mut context), Some(4));
        assert_eq!(store.query(Time(20), &mut context).unwrap().recent, 13);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn counted_replay_excludes_silent_query_forward_and_prune() {
        let config = CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 1 } };
        let mut store = Adapter::create(7, &config, &Time(0));
        let mut context = Context::default();
        store.insert(event(1, 10, 1), &Time(0));
        store.insert(event(2, 20, 2), &Time(0));
        assert_eq!(store.query(Time(20), &mut context).unwrap().recent, 3);
        store.forward(&Time(20), &mut context);
        store.prune();
        assert!(context.effects.is_empty());
        assert_eq!(store.process_until_counted(&Time(20), &mut context), Some(1));
        assert_eq!(context.effects, vec![Time(20)]);
        // Existing replay at the horizon deliberately starts at its predecessor.
        assert_eq!(store.process_until_counted(&Time(20), &mut context), Some(1));
        assert_eq!(store.process_until_counted(&Time(30), &mut context), Some(0));
    }
}
