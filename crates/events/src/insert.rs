use crate::contime_events::{Event, EventHistory, EventKey, Insert, PolicyCall, PolicyDecision, PolicyInsert};

impl Insert {
    /// Returns whether canonical history changed.
    pub const fn changed(&self) -> bool {
        matches!(self, Self::Inserted)
    }
}

impl<E> EventHistory<E>
where
    E: Event,
{
    /// Retains one event at or after the horizon unless its ID is already present.
    ///
    /// Horizon rejection precedes duplicate detection. Either rejected outcome
    /// drops the incoming value and leaves retained history and replay dirtiness
    /// unchanged. Successful insertion updates the dirty replay boundary.
    ///
    /// In-order events append to the ordered deque. Events before the current
    /// tail enter the late-event tree. Identity is independent of timestamp:
    /// a retained event ID always makes a later insertion a no-op.
    pub fn insert(&mut self, event: E) -> Insert {
        match self.insert_inner(event, |_, _| Ok(PolicyDecision::KeepExisting)) {
            PolicyInsert::Inserted => Insert::Inserted,
            PolicyInsert::Duplicate => Insert::Duplicate,
            PolicyInsert::BeforeHorizon => Insert::BeforeHorizon,
            _ => unreachable!("default duplicate policy never replaces an event"),
        }
    }
    /// Inserts with callbacks for admission and duplicate decisions.
    pub fn insert_with_policy<F>(&mut self, event: E, admission_horizon: E::Time, mut policy: F) -> PolicyInsert<E::Time>
    where
        F: FnMut(PolicyCall, &E) -> Result<PolicyDecision<E>, ()>,
    {
        if event.time() < admission_horizon {
            return PolicyInsert::BeforeHorizon;
        }
        self.insert_inner(event, |call, event| {
            let decision = policy(call, event)?;
            match decision {
                PolicyDecision::ReplaceExisting { ref event, ref existing_time }
                    if *existing_time < admission_horizon || event.time() < admission_horizon =>
                {
                    Err(())
                }
                other => Ok(other),
            }
        })
    }
    #[inline]
    fn insert_inner<F>(&mut self, event: E, mut policy: F) -> PolicyInsert<E::Time>
    where
        F: FnMut(PolicyCall, &E) -> Result<PolicyDecision<E>, ()>,
    {
        let key = EventKey::from_event(&event);
        if key.time < self.horizon {
            return PolicyInsert::BeforeHorizon;
        }
        let id = event.event_id();
        if !self.retained_ids.insert(id) {
            return self.on_duplicate(event, id, policy);
        }
        // A default policy is a no-op; custom policies can initialize their context.
        if policy(PolicyCall::Admission, &event).is_err() {
            self.retained_ids.remove(&id);
            return PolicyInsert::InvalidSelection;
        }
        self.record_change(&key);
        self.insert_unique(key, event);
        PolicyInsert::Inserted
    }
    fn on_duplicate<F>(&mut self, incoming: E, id: u128, mut policy: F) -> PolicyInsert<E::Time>
    where
        F: FnMut(PolicyCall, &E) -> Result<PolicyDecision<E>, ()>,
    {
        let (selected, old_time) = match policy(PolicyCall::Duplicate, &incoming) {
            Ok(PolicyDecision::KeepExisting) => return PolicyInsert::Duplicate,
            Ok(PolicyDecision::ReplaceExisting { event, existing_time }) => (event, existing_time),
            Err(()) => return PolicyInsert::InvalidSelection,
        };
        let new_time = selected.time();
        if selected.event_id() != id || old_time < self.horizon || new_time < self.horizon {
            return PolicyInsert::InvalidSelection;
        }
        let old_key = EventKey { time: old_time.clone(), event_id: id };
        let index = self.ordered.partition_point(|(key, _)| key < &old_key);
        if self.ordered.get(index).is_some_and(|(key, _)| *key == old_key) {
            self.ordered.remove(index);
        } else if self.late.remove(&old_key).is_none() {
            return PolicyInsert::InvalidSelection;
        }
        let affected_from = old_time.min(new_time.clone());
        let key = EventKey::from_event(&selected);
        if !self.dirty || affected_from < self.dirty_time {
            self.dirty_time = affected_from.clone();
        }
        self.dirty = true;
        self.insert_unique(key, selected);
        PolicyInsert::Replaced { affected_from, selected_time: new_time }
    }

    #[inline]
    fn record_change(&mut self, key: &EventKey<E::Time>) {
        if self.is_empty() || !self.dirty || key.time < self.dirty_time {
            self.dirty_time = key.time.clone();
        }
        self.dirty = true;
    }
    #[inline]
    fn insert_unique(&mut self, key: EventKey<E::Time>, event: E) {
        if self.ordered.back().is_none_or(|(latest, _)| key > *latest) {
            self.ordered.push_back((key, event));
        } else {
            let previous = self.late.insert(key, event);
            debug_assert!(previous.is_none(), "a unique retained event ID produced a duplicate canonical key");
        }
    }
}

#[cfg(test)]
mod tests {
    use criterion::{BatchSize, Criterion};
    use std::hint::black_box;

    use crate::contime_events::{Event, EventHistory, EventKey, Insert};

    #[derive(Clone)]
    struct TestEvent {
        id: u128,
        time: i64,
    }

    impl Event for TestEvent {
        type Time = i64;

        fn event_id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> Self::Time {
            self.time
        }
    }

    fn event(id: u128, time: i64) -> TestEvent {
        TestEvent { id, time }
    }

    #[test]
    fn history_retains_the_selected_event_ownership_type_directly() {
        let mut history = EventHistory::new();

        history.insert(TestEvent { id: 7, time: 11 });

        let (_key, retained) = history.iter().next().unwrap();
        assert_eq!(retained.id, 7);
        assert_eq!(retained.time, 11);
    }

    #[test]
    fn an_event_after_the_tail_uses_ordered_storage() {
        let mut history = EventHistory::new();

        let inserted = history.insert(event(1, 10));

        assert_eq!(inserted, Insert::Inserted);
        assert!(inserted.changed());
        assert_eq!(history.storage_counts(), (1, 0));
        assert_eq!(history.dirty_time(), &10);
    }

    #[test]
    fn an_event_before_the_tail_uses_late_storage() {
        let mut history = EventHistory::new();
        history.insert(event(2, 20));

        let inserted = history.insert(event(1, 10));

        assert_eq!(inserted, Insert::Inserted);
        assert_eq!(history.storage_counts(), (1, 1));
    }

    #[test]
    fn a_retained_id_is_a_duplicate_even_when_time_changes() {
        let mut history = EventHistory::new();
        history.insert(event(1, 10));

        let duplicate = history.insert(event(1, 100));

        assert_eq!(duplicate, Insert::Duplicate);
        assert!(!duplicate.changed());
        assert_eq!(history.len(), 1);
        assert_eq!(history.latest_key(), Some(&EventKey { time: 10, event_id: 1 }));
    }

    #[test]
    fn a_late_event_moves_dirty_time_back_but_a_duplicate_does_not() {
        let mut history = EventHistory::new();
        history.insert(event(3, 30));
        history.mark_replayed();

        history.insert(event(1, 10));
        assert_eq!(history.dirty_time(), &10);

        history.insert(event(1, 5));
        assert_eq!(history.dirty_time(), &10);
    }

    #[test]
    fn insertion_rejects_only_times_strictly_before_the_horizon() {
        let mut history = EventHistory::with_horizon(10);

        assert_eq!(history.insert(event(1, 9)), Insert::BeforeHorizon);
        assert_eq!(history.insert(event(2, 10)), Insert::Inserted);
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn horizon_rejection_precedes_duplicate_detection() {
        let mut history = EventHistory::with_horizon(0);
        assert_eq!(history.insert(event(1, 10)), Insert::Inserted);
        history.prune_before(&20);

        assert_eq!(history.insert(event(1, 10)), Insert::BeforeHorizon);
    }

    fn ordered_fixture() -> (EventHistory<TestEvent>, Vec<TestEvent>) {
        let events = (0..1_000).map(|value| event(value, value as i64)).collect();
        (EventHistory::with_capacity(1_000), events)
    }

    fn late_fixture() -> (EventHistory<TestEvent>, Vec<TestEvent>) {
        let mut history = EventHistory::with_capacity(1_001);
        history.insert(event(10_000, 10_000));
        let events = (0..1_000).map(|value| event(value, value as i64)).collect();
        (history, events)
    }

    fn duplicate_fixture() -> (EventHistory<TestEvent>, Vec<TestEvent>) {
        let events = (0..1_000).map(|value| event(value, value as i64)).collect::<Vec<_>>();
        let mut history = EventHistory::with_capacity(1_000);
        for event in &events {
            history.insert(event.clone());
        }
        (history, events)
    }

    fn empty_ordered_fixture() -> (EventHistory<TestEvent>, Vec<TestEvent>) {
        let events = (0..1_000).map(|value| event(value, value as i64)).collect();
        (EventHistory::with_horizon(0), events)
    }

    fn benchmark_fixture(criterion: &mut Criterion, name: &str, fixture: fn() -> (EventHistory<TestEvent>, Vec<TestEvent>)) {
        criterion.bench_function(&format!("events/insert/1000_{name}"), |bencher| {
            bencher.iter_batched(
                fixture,
                |(mut history, events)| {
                    for event in events {
                        black_box(history.insert(event));
                    }
                    black_box(history)
                },
                BatchSize::LargeInput,
            );
        });
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_insert() {
        let mut criterion = Criterion::default();

        benchmark_fixture(&mut criterion, "ordered", ordered_fixture);
        benchmark_fixture(&mut criterion, "late", late_fixture);
        benchmark_fixture(&mut criterion, "duplicate", duplicate_fixture);

        criterion.bench_function("events/insert/1000_ordered_unreserved", |bencher| {
            bencher.iter_batched(
                empty_ordered_fixture,
                |(mut history, events)| {
                    for event in events {
                        black_box(history.insert(event));
                    }
                    black_box(history)
                },
                BatchSize::LargeInput,
            );
        });
        criterion.bench_function("events/insert/1000_ordered_reserved_in_timed_path", |bencher| {
            bencher.iter_batched(
                empty_ordered_fixture,
                |(mut history, events)| {
                    history.reserve(events.len());
                    for event in events {
                        black_box(history.insert(event));
                    }
                    black_box(history)
                },
                BatchSize::LargeInput,
            );
        });

        criterion.final_summary();
    }
}

#[cfg(test)]
mod policy_replacement_tests {
    use crate::contime_events::{Event, EventHistory, PolicyCall, PolicyDecision, PolicyInsert};
    #[derive(Clone)]
    struct TestEvent {
        id: u128,
        time: i64,
    }
    impl Event for TestEvent {
        type Time = i64;
        fn event_id(&self) -> u128 {
            self.id
        }
        fn time(&self) -> i64 {
            self.time
        }
    }
    fn admit(call: PolicyCall, _: &TestEvent) -> Result<PolicyDecision<TestEvent>, ()> {
        match call {
            PolicyCall::Admission => Ok(PolicyDecision::KeepExisting),
            PolicyCall::Duplicate => panic!("new ID"),
        }
    }
    fn replace(old: i64, call: PolicyCall, incoming: &TestEvent) -> Result<PolicyDecision<TestEvent>, ()> {
        match call {
            PolicyCall::Duplicate => Ok(PolicyDecision::ReplaceExisting { event: incoming.clone(), existing_time: old }),
            PolicyCall::Admission => panic!("expected duplicate"),
        }
    }
    #[test]
    fn duplicate_keep_never_reads_the_existing_event() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        struct Tracked {
            id: u128,
            reads: Arc<AtomicUsize>,
        }
        impl Event for Tracked {
            type Time = i64;
            fn time(&self) -> i64 {
                10
            }
            fn event_id(&self) -> u128 {
                self.reads.fetch_add(1, Ordering::Relaxed);
                self.id
            }
        }
        let reads = Arc::new(AtomicUsize::new(0));
        let mut h = EventHistory::new();
        h.insert_with_policy(Tracked { id: 1, reads: reads.clone() }, 0, |_, _| Ok(PolicyDecision::KeepExisting));
        let before = reads.load(Ordering::Relaxed);
        let incoming = Tracked { id: 1, reads: Arc::new(AtomicUsize::new(0)) };
        assert!(matches!(h.insert_with_policy(incoming, 0, |_, _| Ok(PolicyDecision::KeepExisting)), PolicyInsert::Duplicate));
        assert_eq!(reads.load(Ordering::Relaxed), before);
    }
    #[test]
    fn replacement_uses_context_time_without_scanning_stored_events() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        #[derive(Clone)]
        struct Tracked {
            id: u128,
            time: i64,
            reads: Arc<AtomicUsize>,
        }
        impl Event for Tracked {
            type Time = i64;
            fn time(&self) -> i64 {
                self.time
            }
            fn event_id(&self) -> u128 {
                self.reads.fetch_add(1, Ordering::Relaxed);
                self.id
            }
        }
        let reads = Arc::new(AtomicUsize::new(0));
        let mut h = EventHistory::new();
        h.insert_with_policy(Tracked { id: 1, time: 20, reads: reads.clone() }, 0, |_, _| Ok(PolicyDecision::KeepExisting));
        let before = reads.load(Ordering::Relaxed);
        let incoming = Tracked { id: 1, time: 10, reads: Arc::new(AtomicUsize::new(0)) };
        let result =
            h.insert_with_policy(incoming, 0, |_, event| Ok(PolicyDecision::ReplaceExisting { event: event.clone(), existing_time: 20 }));
        assert!(matches!(result, PolicyInsert::Replaced { affected_from: 10, .. }));
        assert_eq!(reads.load(Ordering::Relaxed), before);
    }
    #[test]
    fn invalid_existing_time_does_not_remove_the_canonical_event() {
        let mut h = EventHistory::new();
        h.insert_with_policy(TestEvent { id: 1, time: 20 }, 0, admit);
        let result = h.insert_with_policy(TestEvent { id: 1, time: 10 }, 0, |_, event| {
            Ok(PolicyDecision::ReplaceExisting { event: event.clone(), existing_time: 15 })
        });
        assert!(matches!(result, PolicyInsert::InvalidSelection));
        assert_eq!(h.iter().map(|(_, event)| (event.id, event.time)).collect::<Vec<_>>(), [(1, 20)]);
    }
    #[test]
    fn ordered_event_replaced_by_earlier_occurrence() {
        let mut h = EventHistory::new();
        assert!(matches!(h.insert_with_policy(TestEvent { id: 1, time: 20 }, 0, admit), PolicyInsert::Inserted));
        h.insert_with_policy(TestEvent { id: 2, time: 30 }, 0, admit);
        let result = h.insert_with_policy(TestEvent { id: 1, time: 10 }, 0, |c, e| replace(20, c, e));
        assert!(matches!(result, PolicyInsert::Replaced { affected_from: 10, .. }));
        assert_eq!(h.iter().map(|(_, e)| (e.id, e.time)).collect::<Vec<_>>(), [(1, 10), (2, 30)]);
    }
    #[test]
    fn late_event_replaced_by_later_occurrence() {
        let mut h = EventHistory::new();
        h.insert_with_policy(TestEvent { id: 1, time: 20 }, 0, admit);
        h.insert_with_policy(TestEvent { id: 2, time: 10 }, 0, admit);
        let result = h.insert_with_policy(TestEvent { id: 2, time: 30 }, 0, |c, e| replace(10, c, e));
        assert!(matches!(result, PolicyInsert::Replaced { affected_from: 10, .. }));
        assert_eq!(h.iter().map(|(_, e)| (e.id, e.time)).collect::<Vec<_>>(), [(1, 20), (2, 30)]);
    }
    #[test]
    fn pruned_id_is_admitted_as_new() {
        let mut h = EventHistory::new();
        h.insert_with_policy(TestEvent { id: 1, time: 10 }, 0, admit);
        h.prune_before(&20);
        assert!(matches!(h.insert_with_policy(TestEvent { id: 1, time: 20 }, 20, admit), PolicyInsert::Inserted));
    }
    #[test]
    fn invalid_replacement_keeps_original() {
        let mut h = EventHistory::new();
        h.insert_with_policy(TestEvent { id: 1, time: 20 }, 0, admit);
        assert!(matches!(
            h.insert_with_policy(TestEvent { id: 1, time: 30 }, 0, |_, _| Ok(PolicyDecision::ReplaceExisting {
                event: TestEvent { id: 2, time: 30 },
                existing_time: 20
            })),
            PolicyInsert::InvalidSelection
        ));
        assert_eq!(h.iter().map(|(_, e)| (e.id, e.time)).collect::<Vec<_>>(), [(1, 20)]);
    }
}
