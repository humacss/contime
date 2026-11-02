use history::{ContimeModule, Input};
#[derive(ContimeModule)]
#[contime(modules = [contime_domain_one::Module, contime_domain_two::Module])]
pub struct App;
#[test]
fn both_lanes_survive() {
    let a: AppEventLanes = contime_domain_one::First { time: -1, id: 7 }.into();
    let b: AppEventLanes = contime_domain_two::Second { time: -1, id: 7 }.into();
    let mut ids = Vec::new();
    a.snapshot_ids(&mut |id| ids.push(id));
    b.snapshot_ids(&mut |id| ids.push(id));
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
}

#[derive(history::ContimeEvent)]
#[contime(id = renamed_event_id)]
struct RenamedEvent {
    owner: u128,
    #[contime(time)]
    at: i64,
}
fn renamed_event_id(_: &RenamedEvent) -> u128 {
    7
}
#[derive(Clone, Default, history::ContimeSnapshot)]
#[contime(id = |value| value.owner)]
#[contime(events = [RenamedEvent])]
struct RenamedSnapshot {
    owner: u128,
    #[contime(time)]
    at: i64,
}
#[test]
fn timestamp_helpers_resolve_a_renamed_dependency() {
    fn registered<T: history::SnapshotEvents>() {}
    registered::<RenamedSnapshot>();
    let source = RenamedEvent { owner: 9, at: -5 };
    let created = <RenamedSnapshot as history::SnapshotCreate>::create(9, history::EventRef::new(&source));
    assert_eq!(created.owner, 9);
    assert_eq!(history::checkpoints::Event::time(&RenamedEvent { owner: 9, at: -5 }), -5);
    assert_eq!(history::EventId::event_id(&RenamedEvent { owner: 9, at: -5 }), 7);
    let mut s = RenamedSnapshot { owner: 9, at: -5 };
    assert_eq!(history::SnapshotId::snapshot_id(&s), 9);
    history::checkpoints::Snapshot::set_time(&mut s, 10);
    assert_eq!(*history::checkpoints::Snapshot::time(&s), 10);
}

#[derive(history::ContimeModule)]
#[contime(snapshots = [RenamedSnapshot])]
struct RenamedApp;
#[test]
fn snapshot_metadata_resolves_renamed_dependencies_and_inferred_time() {
    fn same_time(_: <RenamedApp as history::Module>::Time) -> i64 {
        0
    }
    assert_eq!(same_time(-5), 0);
    let _: RenamedAppEventLanes = RenamedEvent { owner: 9, at: -5 }.into();
}
impl history::SnapshotApply for RenamedSnapshot {}

#[derive(history::ContimeModule)]
#[contime(modules = [contime_domain_one::GenericModule])]
struct GenericApp;
#[derive(history::ContimeModule)]
#[contime(snapshots = [contime_domain_one::GenericState<u64,8>])]
struct DirectGeneric;
#[test]
fn generic_snapshot_metadata_normalizes_to_concrete_foreign_event_types() {
    let _ = GenericApp;
    let _ = DirectGeneric;
    let _: GenericAppEventLanes =
        contime_domain_one::GenericInput::<u64> { id: 1, owner: 7, at: -5, payload: 0, marker: std::marker::PhantomData }.into();
    let _: DirectGenericEventLanes =
        contime_domain_one::GenericInput::<u64, 8> { id: 1, owner: 7, at: -5, payload: 0, marker: std::marker::PhantomData }.into();
}

#[derive(history::ContimeModule)]
#[contime(snapshots=[contime_domain_one::DefaultedState])]
struct Defaulted;
#[derive(history::ContimeModule)]
#[contime(snapshots=[contime_domain_one::nested::RelativeState])]
struct Relative;
#[test]
fn defaults_and_relative_paths_keep_their_provider_meaning() {
    let _ = Defaulted;
    let _ = Relative;
    let _: DefaultedEventLanes =
        contime_domain_one::Event0::<u64, 8> { id: 1, owner: 7, at: -5, payload: 0, marker: std::marker::PhantomData }.into();
    let _: RelativeEventLanes = contime_domain_one::First { id: 1, time: -5 }.into();
}

#[derive(history::ContimeModule)]
#[contime(modules=[contime_domain_one::ProjectedModule])]
struct ProjectedApp;
#[test]
fn generic_associated_event_arguments_preserve_snapshot_bounds() {
    let _ = ProjectedApp;
    let _: ProjectedAppEventLanes =
        contime_domain_one::GenericInput::<u64> { id: 1, owner: 7, at: -5, payload: 0, marker: std::marker::PhantomData }.into();
}

#[derive(history::ContimeModule)]
#[contime(modules=[contime_domain_one::definitions::Scoped])]
struct Scoped;
#[test]
fn nested_qualified_snapshot_arguments_keep_their_definition_scope() {
    let _ = Scoped;
    let _: ScopedEventLanes =
        contime_domain_one::GenericInput::<u64, 8> { id: 1, owner: 7, at: -5, payload: 0, marker: std::marker::PhantomData }.into();
}
