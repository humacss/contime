//! Worker storage for histories with selectable event occurrences.
use crate::contime_core::checkpoints::{Application, ApplyEvents, ApplyWrapper, Snapshot};
use crate::contime_core::types::History;
use crate::contime_core::{Input, InsertionRejection, RejectionReason, SharedEvent};
use crate::contime_worker::EventInsert;
use std::marker::PhantomData;

/// Snapshot storage applying the individual event types' duplicate policies.
pub struct EventPolicyStorage<I: Input, S: Snapshot<Time = I::Time>, W> {
    snapshot_id: u128,
    horizon: I::Time,
    interval: u64,
    context: std::collections::HashMap<std::any::TypeId, Box<dyn std::any::Any>>,
    affected: Option<I::Time>,
    store: Option<crate::contime_checkpoints::SnapshotStore<S, History<I>>>,
    wrapper: PhantomData<fn() -> W>,
}
impl<I, S, W> crate::contime_worker::SnapshotStore<SharedEvent<I>> for EventPolicyStorage<I, S, W>
where
    I: Input,
    S: Snapshot<Time = I::Time> + ApplyEvents<I>,
    W: ApplyWrapper<S, I>,
{
    type Config = crate::contime_core::CheckpointStorageConfig;
    type Context = W;
    type Time = I::Time;
    type Snapshot = S;
    type Rejection = InsertionRejection;
    fn create(snapshot_id: u128, config: &Self::Config, horizon: &I::Time) -> Self {
        Self {
            snapshot_id,
            horizon: horizon.clone(),
            interval: config.checkpoints.interval,
            context: std::collections::HashMap::new(),
            affected: None,
            store: None,
            wrapper: PhantomData,
        }
    }
    fn event_time(input: &SharedEvent<I>) -> I::Time {
        input.inner.time()
    }
    fn affected_time(&self, time: &I::Time) -> I::Time {
        self.affected.clone().unwrap_or_else(|| time.clone())
    }
    fn earliest_replay_time(&self) -> Option<I::Time> {
        self.store.as_ref().and_then(|store| store.earliest_replay_time())
    }
    fn insert(&mut self, input: SharedEvent<I>, horizon: &I::Time) -> EventInsert<InsertionRejection> {
        let time = input.inner.time();
        let id = input.event_id();
        let reject = |reason| EventInsert { changed: false, rejections: vec![InsertionRejection { event_id: id, reason }] };
        if time < *horizon {
            return reject(RejectionReason::BeforeHistoryHorizon);
        }
        let store = self.store.get_or_insert_with(|| {
            let mut snapshot = S::create(self.snapshot_id, input.inner.as_ref());
            snapshot.set_time(self.horizon.clone());
            crate::contime_checkpoints::SnapshotStore::new(History::with_horizon(self.horizon.clone()), snapshot, self.interval)
        });
        let contexts = &mut self.context;
        let result = store.edit_history_with_result(|history| {
            let result = history.events.insert_with_policy(input, horizon.clone(), |call, incoming| {
                use crate::contime_core::module::DuplicateDecision;
                use crate::contime_events::{PolicyCall, PolicyDecision};
                let event = incoming.inner.as_ref();
                match call {
                    PolicyCall::Admission => {
                        if I::has_custom_duplicate_policies() && event.track_duplicate_admissions() {
                            let state = contexts.entry(event.duplicate_policy_key()).or_insert_with(|| event.create_duplicate_context());
                            event.admit_duplicate_context(state.as_mut());
                        }
                        Ok(PolicyDecision::KeepExisting)
                    }
                    PolicyCall::Duplicate => {
                        let key = event.duplicate_policy_key();
                        let state = contexts.entry(key).or_insert_with(|| event.create_duplicate_context());
                        match event.select_duplicate(state.as_mut()).ok_or(())? {
                            DuplicateDecision::KeepExisting => Ok(PolicyDecision::KeepExisting),
                            DuplicateDecision::ReplaceExisting { event: selected, existing_time } => {
                                Ok(PolicyDecision::ReplaceExisting { event: SharedEvent::from(selected), existing_time })
                            }
                        }
                    }
                }
            });
            let affected = match &result {
                crate::contime_events::PolicyInsert::Inserted => Some(time.clone()),
                crate::contime_events::PolicyInsert::Replaced { affected_from, .. } => Some(affected_from.clone()),
                _ => None,
            };
            (result, affected)
        });
        match result {
            crate::contime_events::PolicyInsert::Inserted => {
                self.affected = Some(time);
                EventInsert { changed: true, rejections: vec![] }
            }
            crate::contime_events::PolicyInsert::Replaced { affected_from, .. } => {
                self.affected = Some(affected_from);
                EventInsert { changed: true, rejections: vec![] }
            }
            crate::contime_events::PolicyInsert::Duplicate => EventInsert { changed: false, rejections: vec![] },
            crate::contime_events::PolicyInsert::BeforeHorizon => reject(RejectionReason::BeforeHistoryHorizon),
            crate::contime_events::PolicyInsert::InvalidSelection => reject(RejectionReason::InvalidDuplicateSelection),
        }
    }
    fn process_until(&mut self, time: &I::Time, context: &mut W) {
        if let Some(store) = &mut self.store {
            let application = Application { snapshot_id: self.snapshot_id, wrapper: std::cell::RefCell::new(context), live: true };
            store.process_until(time.clone(), &application).expect("scheduled processing must have a valid checkpoint");
        }
    }
    #[cfg(feature = "monitor")]
    fn process_until_counted(&mut self, time: &I::Time, context: &mut W) -> Option<u64> {
        Some(self.store.as_mut().map_or(0, |store| {
            let application = Application { snapshot_id: self.snapshot_id, wrapper: std::cell::RefCell::new(context), live: true };
            store.process_until_counted(time.clone(), &application).expect("scheduled processing must have a valid checkpoint")
        }))
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
            let application = Application { snapshot_id: self.snapshot_id, wrapper: std::cell::RefCell::new(context), live: false };
            store
                .forward(horizon.clone(), &application, |snapshot, time, application| {
                    application.wrapper.borrow_mut().retain_snapshot(snapshot, time)
                })
                .expect("forwarding must have a predecessor checkpoint");
        }
        self.horizon = horizon.clone();
    }
    fn prune(&mut self) {
        if let Some(store) = &mut self.store {
            if !self.context.is_empty() {
                for (_, event) in store.events().events.iter().take_while(|(key, _)| key.time < self.horizon) {
                    if let Some(context) = self.context.get_mut(&event.inner.duplicate_policy_key()) {
                        event.inner.prune_duplicate_context(context.as_mut());
                    }
                }
            }
            store.prune();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contime_core::{
        checkpoints::{ApplyBatch, CheckpointConfig},
        types::testing::Time,
        Input,
    };
    use crate::contime_worker::SnapshotStore;
    #[derive(Clone)]
    struct TestEvent {
        id: u128,
        time: Time,
    }
    impl crate::contime_checkpoints::Event for TestEvent {
        type Time = Time;
        fn time(&self) -> Time {
            self.time
        }
    }
    struct Context {
        count: usize,
        time: Time,
    }
    impl Input for TestEvent {
        const TRACKS_DUPLICATE_ADMISSIONS: bool = true;
        fn event_id(&self) -> u128 {
            self.id
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(1);
        }
        fn create_duplicate_context(&self) -> Box<dyn std::any::Any> {
            Box::new(Context { count: 0, time: Time(20) })
        }
        fn prune_duplicate_context(&self, context: &mut dyn std::any::Any) {
            context.downcast_mut::<Context>().unwrap().count = 0;
        }
        fn track_duplicate_admissions(&self) -> bool {
            self.id != 2
        }
        fn select_duplicate(&self, context: &mut dyn std::any::Any) -> Option<crate::contime_core::module::DuplicateDecision<Self>> {
            let context = context.downcast_mut::<Context>()?;
            context.count += 1;
            Some(if context.count >= 2 {
                crate::contime_core::module::DuplicateDecision::ReplaceExisting {
                    existing_time: std::mem::replace(&mut context.time, self.time),
                    event: self.clone(),
                }
            } else {
                crate::contime_core::module::DuplicateDecision::KeepExisting
            })
        }
    }
    #[derive(Clone)]
    struct State {
        time: Time,
        values: Vec<Time>,
    }
    impl Snapshot for State {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.time
        }
        fn set_time(&mut self, time: Time) {
            self.time = time;
        }
    }
    impl ApplyEvents<TestEvent> for State {
        fn create(_: u128, _: &TestEvent) -> Self {
            Self { time: Time(0), values: vec![] }
        }
        fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, TestEvent>) {
            self.values.extend(batch.events.map(|event| event.time));
        }
    }
    type Store = EventPolicyStorage<TestEvent, State, ()>;
    fn config() -> crate::contime_core::CheckpointStorageConfig {
        crate::contime_core::CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 1 } }
    }
    fn input(time: i64) -> SharedEvent<TestEvent> {
        TestEvent { id: 1, time: Time(time) }.into()
    }
    #[test]
    fn duplicate_context_is_per_snapshot_and_same_time_still_calls_policy() {
        let mut a = Store::create(1, &config(), &Time(0));
        let mut b = Store::create(2, &config(), &Time(0));
        for store in [&mut a, &mut b] {
            assert!(store.insert(input(20), &Time(0)).changed);
        }
        assert!(!a.insert(input(20), &Time(0)).changed);
        assert!(a.insert(input(10), &Time(0)).changed);
        assert!(!b.insert(input(10), &Time(0)).changed);
        assert_eq!(a.query(Time(30), &mut ()).unwrap().values, [Time(10)]);
        assert_eq!(b.query(Time(30), &mut ()).unwrap().values, [Time(20)]);
    }
    #[test]
    fn replacement_without_admission_tracking_replays_from_the_old_event() {
        let mut store = Store::create(1, &config(), &Time(0));
        let event = |time| SharedEvent::from(TestEvent { id: 2, time: Time(time) });
        assert!(store.insert(event(20), &Time(0)).changed);
        store.process_until(&Time(30), &mut ());
        assert!(!store.insert(event(20), &Time(0)).changed);
        assert!(store.insert(event(10), &Time(0)).changed);
        assert_eq!(store.query(Time(15), &mut ()).unwrap().values, [Time(10)]);
    }
    #[test]
    fn replacement_replays_when_selected_earlier_and_then_later() {
        let mut store = Store::create(1, &config(), &Time(0));
        assert!(store.insert(input(20), &Time(0)).changed);
        store.process_until(&Time(30), &mut ());
        assert!(!store.insert(input(20), &Time(0)).changed);
        assert!(store.insert(input(10), &Time(0)).changed);
        assert_eq!(store.query(Time(15), &mut ()).unwrap().values, [Time(10)]);
        store.process_until(&Time(30), &mut ());
        assert!(store.insert(input(25), &Time(0)).changed);
        assert!(store.query(Time(15), &mut ()).unwrap().values.is_empty());
        assert_eq!(store.query(Time(30), &mut ()).unwrap().values, [Time(25)]);
    }
    #[test]
    fn pruning_forgets_removed_ids_and_keeps_snapshot_state() {
        let mut store = Store::create(1, &config(), &Time(0));
        assert!(store.insert(input(10), &Time(0)).changed);
        store.process_until(&Time(20), &mut ());
        store.forward(&Time(15), &mut ());
        store.prune();
        assert!(!store.insert(input(14), &Time(15)).changed);
        assert!(store.insert(input(20), &Time(15)).changed);
        assert_eq!(store.query(Time(25), &mut ()).unwrap().values, [Time(10), Time(20)]);
    }
    #[test]
    fn pruning_notifies_policy_context_before_the_id_is_reused() {
        let mut store = Store::create(1, &config(), &Time(0));
        assert!(store.insert(input(10), &Time(0)).changed);
        assert!(!store.insert(input(10), &Time(0)).changed);
        store.process_until(&Time(20), &mut ());
        store.forward(&Time(15), &mut ());
        store.prune();
        assert!(store.insert(input(20), &Time(15)).changed);
        assert!(!store.insert(input(30), &Time(15)).changed);
        assert_eq!(store.query(Time(30), &mut ()).unwrap().values, [Time(10), Time(20)]);
    }
    #[test]
    fn moving_an_event_past_an_in_flight_prune_is_rejected() {
        let mut store = Store::create(1, &config(), &Time(0));
        assert!(store.insert(input(10), &Time(0)).changed);
        store.process_until(&Time(30), &mut ());
        assert!(!store.insert(input(20), &Time(0)).changed);
        let result = store.insert(input(20), &Time(15));
        assert!(!result.changed);
        assert_eq!(result.rejections[0].reason, RejectionReason::InvalidDuplicateSelection);
        assert_eq!(store.query(Time(30), &mut ()).unwrap().values, [Time(10)]);
    }
}
