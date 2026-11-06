use contime::{ContimeEvent, DuplicateDecision, EventDuplicatePolicy};

#[derive(ContimeEvent)]
struct DefaultEvent {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
}
#[derive(Clone, ContimeEvent)]
#[contime(duplicate_policy = manual)]
struct CustomEvent {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
}
struct Context {
    decisions: u32,
    times: std::collections::HashMap<u128, i64>,
}
impl EventDuplicatePolicy for CustomEvent {
    type Context = Context;
    fn create_context() -> Context {
        Context { decisions: 0, times: std::collections::HashMap::new() }
    }
    fn on_admit(event: &Self, context: &mut Context) {
        context.times.insert(event.id, event.time);
    }
    fn select_canonical(incoming: &Self, context: &mut Context) -> DuplicateDecision<Self> {
        context.decisions += 1;
        let time = context.times.get_mut(&incoming.id).unwrap();
        if incoming.time < *time {
            let existing_time = std::mem::replace(time, incoming.time);
            DuplicateDecision::ReplaceExisting { existing_time, event: incoming.clone() }
        } else {
            DuplicateDecision::KeepExisting
        }
    }
}
#[test]
fn default_and_custom_event_policies_are_defined_on_the_event() {
    let _current = DefaultEvent { id: 1, time: 20 };
    let incoming = DefaultEvent { id: 1, time: 10 };
    let mut default_context = DefaultEvent::create_context();
    assert!(matches!(DefaultEvent::select_canonical(&incoming, &mut default_context), DuplicateDecision::KeepExisting));
    let current = CustomEvent { id: 1, time: 20 };
    let incoming = CustomEvent { id: 1, time: 10 };
    let mut context = CustomEvent::create_context();
    CustomEvent::on_admit(&current, &mut context);
    assert!(
        matches!(CustomEvent::select_canonical(&incoming, &mut context), DuplicateDecision::ReplaceExisting { existing_time: 20, event } if event.time == 10)
    );
    assert_eq!(context.decisions, 1);
}

#[derive(Clone, Default, contime::ContimeSnapshot)]
#[contime(events = [DefaultEvent, CustomEvent])]
struct List {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
    values: Vec<i64>,
}
impl contime::SnapshotApply for List {
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        self.values.extend(batch.events::<DefaultEvent>().map(|event| event.time));
        self.values.extend(batch.events::<CustomEvent>().map(|event| event.time));
    }
}
#[derive(contime::ContimeModule)]
#[contime(snapshots = [List])]
struct App;
#[test]
fn managed_module_uses_the_policy_of_each_event_type() {
    let core = contime::ConTime::<App>::start(crate::config()).unwrap();
    core.apply([DefaultEvent { id: 1, time: 20 }.into(), CustomEvent { id: 2, time: 20 }.into()]).unwrap();
    core.advance_to(30).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    core.apply([DefaultEvent { id: 1, time: 10 }.into(), CustomEvent { id: 2, time: 10 }.into()]).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let states = core.query_at(15, [1, 2]).unwrap();
    assert_eq!(states.len(), 2);
    let mut values = states
        .iter()
        .map(|state| match &**state {
            AppSnapshotLanes::List(list) => (list.id, list.values.clone()),
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|state| state.0);
    assert_eq!(values, vec![(1, vec![]), (2, vec![10])]);
    core.shutdown();
}

#[derive(Clone)]
struct Handwritten {
    id: u128,
    time: i64,
}
impl contime::checkpoints::Event for Handwritten {
    type Time = i64;
    fn time(&self) -> i64 {
        self.time
    }
}
impl contime::EventId for Handwritten {
    fn event_id(&self) -> u128 {
        self.id
    }
}
impl EventDuplicatePolicy for Handwritten {
    type Context = std::collections::HashMap<u128, i64>;
    fn create_context() -> Self::Context {
        Self::Context::new()
    }
    fn on_admit(event: &Self, context: &mut Self::Context) {
        context.insert(event.id, event.time);
    }
    fn select_canonical(incoming: &Self, context: &mut Self::Context) -> DuplicateDecision<Self> {
        if incoming.time < context[&incoming.id] {
            let existing_time = context.insert(incoming.id, incoming.time).unwrap();
            DuplicateDecision::ReplaceExisting { existing_time, event: incoming.clone() }
        } else {
            DuplicateDecision::KeepExisting
        }
    }
}

#[derive(Clone, Default, contime::ContimeSnapshot)]
#[contime(events = [Handwritten])]
struct ManualList {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
    values: Vec<i64>,
}
impl contime::SnapshotApply for ManualList {
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        self.values.extend(batch.events::<Handwritten>().map(|event| event.time));
    }
}
#[derive(contime::ContimeModule)]
#[contime(snapshots = [ManualList])]
struct ManualApp;
#[test]
fn handwritten_event_policy_is_used_by_the_module() {
    let core = contime::ConTime::<ManualApp>::start(crate::config()).unwrap();
    core.apply([Handwritten { id: 1, time: 20 }.into()]).unwrap();
    core.advance_to(30).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    core.apply([Handwritten { id: 1, time: 10 }.into()]).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let states = core.query_at(15, [1]).unwrap();
    let ManualAppSnapshotLanes::ManualList(list) = &*states[0];
    assert_eq!(list.values, vec![10]);
    core.shutdown();
}

#[test]
fn manual_module_replays_a_custom_policy_selection() {
    let mut core = contime::manual::ManualConTime::<App>::new(contime::manual::ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(32).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 1 },
    });
    core.apply([CustomEvent { id: 3, time: 20 }.into()]).unwrap();
    core.advance_to(30).unwrap();
    while core.process().unwrap() {}
    core.apply([CustomEvent { id: 3, time: 10 }.into()]).unwrap();
    let result = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = result.clone();
    core.query_at(15, [3], move |snapshots| {
        let AppSnapshotLanes::List(list) = &*snapshots[0];
        *output.borrow_mut() = Some(list.values.clone());
    })
    .unwrap();
    while core.process().unwrap() {}
    assert_eq!(*result.borrow(), Some(vec![10]));
    assert!(core.take_rejections().is_empty());
}

// Metadata is deliberately excluded from this example's event ID.
#[derive(contime::ContimeEvent)]
#[contime(duplicate_policy = manual)]
struct ConstructedEvent {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
    metadata: u32,
}
impl EventDuplicatePolicy for ConstructedEvent {
    type Context = ();
    const TRACK_ADMISSIONS: bool = false;
    fn create_context() {}
    fn select_canonical(incoming: &Self, _: &mut ()) -> DuplicateDecision<Self> {
        DuplicateDecision::ReplaceExisting {
            existing_time: incoming.time + i64::from(incoming.metadata == 98),
            event: Self { id: incoming.id + u128::from(incoming.metadata == 99), time: incoming.time, metadata: incoming.metadata },
        }
    }
}
#[derive(Clone, Default, contime::ContimeSnapshot)]
#[contime(events = [ConstructedEvent])]
struct MetadataList {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
    values: Vec<u32>,
}
impl contime::SnapshotApply for MetadataList {
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        self.values.extend(batch.events::<ConstructedEvent>().map(|event| event.metadata));
    }
}
#[derive(contime::ContimeModule)]
#[contime(snapshots = [MetadataList])]
struct MetadataApp;
#[test]
fn replacement_at_the_same_time_replays_the_returned_non_clone_event() {
    let core = contime::ConTime::<MetadataApp>::start(crate::config()).unwrap();
    core.apply([ConstructedEvent { id: 9, time: 10, metadata: 1 }.into()]).unwrap();
    core.advance_to(20).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    core.apply([ConstructedEvent { id: 9, time: 10, metadata: 2 }.into()]).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let snapshots = core.query_at(20, [9]).unwrap();
    let MetadataAppSnapshotLanes::MetadataList(list) = &*snapshots[0];
    assert_eq!(list.values, [2]);
    core.shutdown();
}

#[test]
fn replacement_with_a_different_id_is_rejected_without_changing_history() {
    let core = contime::ConTime::<MetadataApp>::start(crate::config()).unwrap();
    core.apply([ConstructedEvent { id: 9, time: 10, metadata: 1 }.into()]).unwrap();
    core.advance_to(20).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    core.apply([ConstructedEvent { id: 9, time: 10, metadata: 99 }.into()]).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(core.errors().try_recv().unwrap().reason, contime::RejectionReason::InvalidDuplicateSelection);
    let snapshots = core.query_at(20, [9]).unwrap();
    let MetadataAppSnapshotLanes::MetadataList(list) = &*snapshots[0];
    assert_eq!(list.values, [1]);
    core.shutdown();
}

#[test]
fn wrong_existing_time_is_rejected_without_changing_history() {
    let core = contime::ConTime::<MetadataApp>::start(crate::config()).unwrap();
    core.apply([ConstructedEvent { id: 9, time: 10, metadata: 1 }.into()]).unwrap();
    core.advance_to(20).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    core.apply([ConstructedEvent { id: 9, time: 10, metadata: 98 }.into()]).unwrap();
    core.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    assert_eq!(core.errors().try_recv().unwrap().reason, contime::RejectionReason::InvalidDuplicateSelection);
    let snapshots = core.query_at(20, [9]).unwrap();
    let MetadataAppSnapshotLanes::MetadataList(list) = &*snapshots[0];
    assert_eq!(list.values, [1]);
    core.shutdown();
}

#[derive(Clone, contime::ContimeEvent)]
#[contime(duplicate_policy = manual)]
struct RetainedEvent {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
}
impl EventDuplicatePolicy for RetainedEvent {
    type Context = (usize, i64);
    fn create_context() -> Self::Context {
        (0, 0)
    }
    fn on_admit(event: &Self, context: &mut Self::Context) {
        context.1 = event.time;
    }
    fn prune_context(_event: &Self, context: &mut Self::Context) {
        *context = (0, 0);
    }
    fn select_canonical(incoming: &Self, context: &mut Self::Context) -> DuplicateDecision<Self> {
        context.0 += 1;
        if context.0 > 1 {
            DuplicateDecision::ReplaceExisting { existing_time: std::mem::replace(&mut context.1, incoming.time), event: incoming.clone() }
        } else {
            DuplicateDecision::KeepExisting
        }
    }
}
#[derive(Clone, Default, contime::ContimeSnapshot)]
#[contime(events = [RetainedEvent])]
struct RetainedList {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
    values: Vec<i64>,
}
impl contime::SnapshotApply for RetainedList {
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        self.values.extend(batch.events::<RetainedEvent>().map(|e| e.time));
    }
}
#[derive(contime::ContimeModule)]
#[contime(snapshots = [RetainedList])]
struct RetainedApp;
#[test]
fn pruned_event_notifies_derived_policy_context_before_id_reuse() {
    let mut core = contime::manual::ManualConTime::<RetainedApp>::new(contime::manual::ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(32).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 1 },
    });
    core.apply([RetainedEvent { id: 1, time: 10 }.into()]).unwrap();
    core.apply([RetainedEvent { id: 1, time: 10 }.into()]).unwrap();
    core.advance_to(20).unwrap();
    while core.process().unwrap() {}
    core.prune_before(15).unwrap();
    while core.process().unwrap() {}
    core.apply([RetainedEvent { id: 1, time: 20 }.into()]).unwrap();
    core.apply([RetainedEvent { id: 1, time: 30 }.into()]).unwrap();
    let result = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = result.clone();
    core.query_at(30, [1], move |snapshots| {
        let RetainedAppSnapshotLanes::RetainedList(list) = &*snapshots[0];
        *output.borrow_mut() = Some(list.values.clone());
    })
    .unwrap();
    while core.process().unwrap() {}
    assert_eq!(*result.borrow(), Some(vec![10, 20]));
    assert!(core.take_rejections().is_empty());
}
