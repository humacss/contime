use contime::{ContimeEvent, ContimeSnapshot, Input, Snapshot, SnapshotEvent};

pub trait TestTarget: Clone + std::fmt::Debug + Default + PartialEq + Eq + Send + Sync + 'static {}

impl<T> TestTarget for T where T: Clone + std::fmt::Debug + Default + PartialEq + Eq + Send + Sync + 'static {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnGenericValueChanged<T: TestTarget> {
    snapshot_id: u128,
    time: i64,
    value: T,
}

impl<T: TestTarget> Input for OnGenericValueChanged<T> {
    type Time = i64;

    fn id(&self) -> u128 {
        self.snapshot_id
    }

    fn time(&self) -> Self::Time {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        32
    }
}

impl<T: TestTarget> contime::Event for OnGenericValueChanged<T> {}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [OnGenericValueChanged<T>],
    id = [snapshot_id],
    bytes = 32,
    apply = {
        for event in batch.events {
            let GenericValueAtEvent::OnGenericValueChanged(event) = event;
            self.value = event.value.clone();
        }
        self.time = batch.time;
    }
)]
pub struct GenericValueAt<T: TestTarget> {
    snapshot_id: u128,
    time: i64,
    value: T,
}

#[test]
fn snapshot_derive_preserves_generic_target_type() {
    let event = OnGenericValueChanged { snapshot_id: 7, time: 11, value: String::from("account:7") };
    let event = GenericValueAtEvent::from(event);
    let source = OnGenericValueChanged { snapshot_id: 7, time: 11, value: String::from("account:7") };
    let mut snapshot = GenericValueAt::default();
    source.set_snapshot_identity(&mut snapshot);
    let events = [&event];

    contime::ApplyEvents::apply_events(
        &mut snapshot,
        contime::ApplyBatch { snapshot_id: 7, time: 11, history_input_count: 1, events: &events },
    );

    assert_eq!("account:7", snapshot.value, "generic snapshot derive did not retain the consumer target value");
}

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 32)]
pub struct OnDerivedValueChanged {
    event_id: u128,
    entity_id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [OnDerivedValueChanged],
    id = [entity_id],
    bytes = 64 + self.retained_input_ids.capacity() as u64 * 16,
    compact = {
        self.retained_input_ids.clear();
        self.retained_input_ids.shrink_to_fit();
    },
    apply = {
        for event in batch.events {
            match event {
                DerivedValueAtEvent::OnDerivedValueChanged(event) => {
                    self.entity_id = event.entity_id;
                    self.value = event.value;
                    self.retained_input_ids.push(event.event_id);
                }
            }
        }
        self.time = batch.time;
    }
)]
pub struct DerivedValueAt {
    entity_id: u128,
    time: i64,
    value: i32,
    retained_input_ids: Vec<u128>,
}

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 32)]
pub struct OnFragmentAlphaChanged {
    event_id: u128,
    entity_id: u128,
    time: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 32)]
pub struct OnFragmentBetaChanged {
    event_id: u128,
    entity_id: u128,
    time: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [OnFragmentAlphaChanged, OnFragmentBetaChanged],
    id = [entity_id],
    bytes = 32,
    apply = {
        self.time = batch.time;
    }
)]
struct FragmentMetadataAt {
    entity_id: u128,
    time: i64,
    value: i32,
}

macro_rules! assert_concrete_fragment_routes {
    (
        @append
        snapshots { FragmentMetadataAt, }
        event_routes {
            OnFragmentAlphaChanged(OnFragmentAlphaChanged) [key = "OnFragmentAlphaChanged"] => FragmentMetadataAtEvent => [FragmentMetadataAt],
            OnFragmentBetaChanged(OnFragmentBetaChanged) [key = "OnFragmentBetaChanged"] => FragmentMetadataAtEvent => [FragmentMetadataAt],
        }
        fragments []
    ) => {
        fn assert_fragment_routes_compile() {}
    };
}

__ao_snapshot_fragment_FragmentMetadataAt! {
    @append
    snapshots {}
    event_routes {}
    fragments [assert_concrete_fragment_routes]
}

contime::lanes! {
    mod derived_lanes;
    snapshots [DerivedValueAt];
    routes [
        OnDerivedValueChanged => [DerivedValueAt],
    ];
}

#[test]
fn derives_generate_event_snapshot_and_lanes() {
    let contime = derived_lanes::Contime::new(1, 2_048);

    contime.apply([OnDerivedValueChanged { event_id: 10, entity_id: 7, time: 5, value: 99 }].map(Into::into)).expect("event should apply");

    let snapshot: DerivedValueAt =
        contime.query_at(6, &[7]).expect("query should succeed").pop().flatten().expect("snapshot should exist").into();

    assert_eq!(7, snapshot.id());
    assert_eq!(99, snapshot.value);
}

#[test]
fn derived_snapshot_compaction_is_delegated_through_generated_lanes() {
    let contime = derived_lanes::Contime::with_history_horizon(1, 2_048, 10);
    contime.apply([OnDerivedValueChanged { event_id: 10, entity_id: 7, time: 5, value: 99 }].map(Into::into)).expect("event should apply");

    contime.advance_to(20).expect("the history horizon should advance");

    let snapshot: DerivedValueAt =
        contime.query_at(20, &[7]).expect("query should succeed").pop().flatten().expect("snapshot should exist").into();
    assert_eq!(snapshot.value, 99);
    assert!(snapshot.retained_input_ids.is_empty());
}

#[test]
fn derived_event_route_initializes_only_snapshot_identity() {
    let input = derived_lanes::InputLanes::from(OnDerivedValueChanged { event_id: 10, entity_id: 7, time: 5, value: 99 });

    let snapshot_ids = <derived_lanes::InputLanes as contime::InputLanes<derived_lanes::SnapshotLanes>>::snapshot_ids(&input);
    let snapshot: DerivedValueAt = <derived_lanes::SnapshotLanes as contime::SnapshotLanes>::materialize(snapshot_ids[0], &input)
        .expect("derived event should materialize its snapshot lane")
        .into();

    assert_eq!(snapshot.entity_id, 7, "derived identity setter initialized the wrong snapshot ID");
    assert_eq!(snapshot.time, 0, "derived identity setter copied event time into clean snapshot state");
    assert_eq!(snapshot.value, 0, "derived identity setter copied event payload into clean snapshot state");
}

#[test]
fn snapshot_fragment_exposes_each_concrete_event_route() {
    assert_fragment_routes_compile();
}
