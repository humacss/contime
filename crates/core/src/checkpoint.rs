use crate::checkpoints::{Application, ApplyEvents, ApplyWrapper, Snapshot};
use crate::types::{CheckpointStorage, CheckpointStorageConfig, History};
use crate::{Input, RejectionMessage, RejectionReason, SharedEvent};
use contime_checkpoints::Insert;
use contime_worker::EventInsert;
use std::marker::PhantomData;

impl<I, S, W> contime_worker::SnapshotStore<SharedEvent<I>> for CheckpointStorage<I, S, W>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I>,
    W: ApplyWrapper<S, I>,
{
    type Config = CheckpointStorageConfig;
    type Context = W;
    type Time = I::Time;
    type Snapshot = S;
    type Rejection = RejectionMessage<RejectionReason>;

    fn create(snapshot_id: u128, config: &Self::Config, horizon: &Self::Time) -> Self {
        Self { snapshot_id, horizon: horizon.clone(), interval: config.checkpoints.interval, store: None, wrapper: PhantomData }
    }

    fn event_time(input: &SharedEvent<I>) -> I::Time {
        input.inner.time()
    }

    fn insert(&mut self, input: SharedEvent<I>, admission_horizon: &I::Time) -> EventInsert<Self::Rejection> {
        let event_id = input.event_id();
        if input.inner.time() < *admission_horizon {
            return EventInsert {
                changed: false,
                rejections: vec![RejectionMessage { event_id, reason: RejectionReason::BeforeHistoryHorizon }],
            };
        }
        let store = self.store.get_or_insert_with(|| {
            let mut snapshot = S::create(self.snapshot_id, input.inner.as_ref());
            snapshot.set_time(self.horizon.clone());
            contime_checkpoints::SnapshotStore::new(History::with_horizon(self.horizon.clone()), snapshot, self.interval)
        });
        match store.insert(input) {
            Insert::Inserted => EventInsert { changed: true, rejections: vec![] },
            Insert::Duplicate => EventInsert { changed: false, rejections: vec![] },
            Insert::BeforeHorizon => EventInsert {
                changed: false,
                rejections: vec![RejectionMessage { event_id, reason: RejectionReason::BeforeHistoryHorizon }],
            },
        }
    }

    fn process_until(&mut self, time: &I::Time, context: &mut W) {
        if let Some(store) = &mut self.store {
            let application = Application { snapshot_id: self.snapshot_id, wrapper: std::cell::RefCell::new(context), live: true };
            store.process_until(time.clone(), &application).expect("scheduled processing must have a valid checkpoint");
        }
    }

    fn query(&mut self, time: I::Time, context: &mut W) -> Option<Box<S>> {
        let application = Application { snapshot_id: self.snapshot_id, wrapper: std::cell::RefCell::new(context), live: false };
        self.store.as_mut()?.query(time, &application).ok()
    }

    fn query_events(&self, from: &I::Time, to: &I::Time) -> Vec<SharedEvent<I>> {
        self.store.as_ref().map_or_else(Vec::new, |store| store.events().events.clone_between(from, to))
    }

    fn forward(&mut self, horizon: &I::Time, context: &mut W) {
        if horizon <= &self.horizon {
            return;
        }
        if let Some(store) = &mut self.store {
            // Reconstruct retained state without republishing historical effects.
            // Consumer compaction still runs through the forwarding hook below.
            let application = Application { snapshot_id: self.snapshot_id, wrapper: std::cell::RefCell::new(context), live: false };
            store
                .forward(horizon.clone(), &application, |snapshot, time, application| {
                    application.wrapper.borrow_mut().retain_snapshot(snapshot, time);
                })
                .expect("forwarding an increasing horizon must have a predecessor checkpoint");
        }
        self.horizon = horizon.clone();
    }

    fn prune(&mut self) {
        if let Some(store) = &mut self.store {
            store.prune();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoints::{ApplyBatch, ApplyInner, CheckpointConfig, EventBatch};
    use crate::types::testing::Time;
    use contime_worker::SnapshotStore;

    struct TestEvent {
        id: u128,
        time: Time,
        value: i64,
    }
    impl contime_checkpoints::Event for TestEvent {
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
            let mut events = batch.events.take(self.limit.unwrap_or(usize::MAX));
            inner.apply_event_batch(EventBatch { snapshot_id: batch.snapshot_id, time: batch.time, events: &mut events });
        }
        fn retain_snapshot(&mut self, snapshot: &mut TestSnapshot, time: &Time) {
            snapshot.compacted += std::mem::take(&mut snapshot.recent);
            self.forwards.push(*time);
        }
    }
    type Adapter = CheckpointStorage<TestEvent, TestSnapshot, Context>;
    fn event(id: u128, time: i64, value: i64) -> SharedEvent<TestEvent> {
        crate::input::prepare_inputs(vec![TestEvent { id, time: Time(time), value }]).pop().unwrap()
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
}
