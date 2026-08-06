//! Verifies that independently defined snapshot/event fragments compose into one lane universe.

use contime::{
    ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, ContimeEvent, ContimeSnapshot, Event, Input, InputBatch, Snapshot, SnapshotEvent,
};

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 32)]
pub struct OnLegacyValueChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    value: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [OnLegacyValueChanged],
    id = [entity_id],
    bytes = 32,
    apply = {
        for event in batch.events {
            let LegacyValueAtEvent::OnLegacyValueChanged(event) = event;
            self.value = event.value;
        }
    }
)]
struct LegacyValueAt {
    entity_id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 32)]
pub struct OnFragmentValueChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    value: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [OnFragmentValueChanged],
    id = [entity_id],
    bytes = 32,
    apply = {
        for event in batch.events {
            let FragmentValueAtEvent::OnFragmentValueChanged(event) = event;
            self.value = event.value;
        }
    }
)]
struct FragmentValueAt {
    entity_id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 32)]
pub struct OnAlphaChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    alpha: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [OnAlphaChanged],
    id = [entity_id],
    bytes = 32,
    apply = {
        for event in batch.events {
            let AlphaAtEvent::OnAlphaChanged(event) = event;
            self.alpha = event.alpha;
        }
    }
)]
struct AlphaAt {
    entity_id: u128,
    time: i64,
    alpha: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, ContimeEvent)]
#[contime_event(id = self.event_id, time = self.time, bytes = 32)]
pub struct OnBetaChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    beta: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, ContimeSnapshot)]
#[contime_snapshot(
    events = [OnBetaChanged],
    id = [entity_id],
    bytes = 32,
    apply = {
        for event in batch.events {
            let BetaAtEvent::OnBetaChanged(event) = event;
            self.beta = event.beta;
        }
    }
)]
struct BetaAt {
    entity_id: u128,
    time: i64,
    beta: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnSharedValueChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    value: i32,
}

impl Input for OnSharedValueChanged {
    type Time = i64;

    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        32
    }
}

impl Event for OnSharedValueChanged {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SharedSourceAtEvent {
    OnSharedValueChanged(OnSharedValueChanged),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SharedMirrorAtEvent {
    OnSharedValueChanged(OnSharedValueChanged),
}

macro_rules! impl_shared_event_lane {
    ($lane:ty) => {
        impl Input for $lane {
            type Time = i64;

            fn id(&self) -> u128 {
                match self {
                    Self::OnSharedValueChanged(event) => event.id(),
                }
            }

            fn time(&self) -> i64 {
                match self {
                    Self::OnSharedValueChanged(event) => event.time(),
                }
            }

            fn conservative_size(&self) -> u64 {
                32
            }
        }

        impl Event for $lane {}

        impl From<OnSharedValueChanged> for $lane {
            fn from(event: OnSharedValueChanged) -> Self {
                Self::OnSharedValueChanged(event)
            }
        }
    };
}

impl_shared_event_lane!(SharedSourceAtEvent);
impl_shared_event_lane!(SharedMirrorAtEvent);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SharedSourceAt {
    entity_id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SharedMirrorAt {
    entity_id: u128,
    time: i64,
    mirrored: i32,
}

impl Snapshot for SharedSourceAt {
    type Time = i64;
    type Input = SharedSourceAtEvent;

    fn id(&self) -> u128 {
        30_000 + self.entity_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        32
    }
}

impl Snapshot for SharedMirrorAt {
    type Time = i64;
    type Input = SharedMirrorAtEvent;

    fn id(&self) -> u128 {
        40_000 + self.entity_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        32
    }
}

impl SnapshotEvent<SharedSourceAt> for OnSharedValueChanged {
    fn snapshot_id(&self) -> u128 {
        30_000 + self.entity_id
    }

    fn set_snapshot_identity(&self, snapshot: &mut SharedSourceAt) {
        snapshot.entity_id = self.entity_id;
    }
}

impl SnapshotEvent<SharedMirrorAt> for OnSharedValueChanged {
    fn snapshot_id(&self) -> u128 {
        40_000 + self.entity_id
    }

    fn set_snapshot_identity(&self, snapshot: &mut SharedMirrorAt) {
        snapshot.entity_id = self.entity_id;
    }
}

impl ApplyEvents<SharedSourceAtEvent> for SharedSourceAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, SharedSourceAtEvent>) {
        for event in batch.events {
            let SharedSourceAtEvent::OnSharedValueChanged(event) = event;
            self.value = event.value;
        }
    }
}

impl ApplyEvents<SharedMirrorAtEvent> for SharedMirrorAt {
    fn apply_events(&mut self, batch: ApplyBatch<'_, SharedMirrorAtEvent>) {
        for event in batch.events {
            let SharedMirrorAtEvent::OnSharedValueChanged(event) = event;
            self.mirrored = event.value;
        }
    }
}

contime::lanes! {
    mod legacy_lanes;
    snapshots [LegacyValueAt];
    routes [OnLegacyValueChanged => [LegacyValueAt]];
}

#[derive(Default)]
struct CountAppliedBatches(usize);

impl ApplyWrapper<distinct_fragment_lanes::SnapshotLanes> for CountAppliedBatches {
    fn apply_input_batch_wrapper(
        &mut self,
        snapshot: &mut distinct_fragment_lanes::SnapshotLanes,
        batch: InputBatch<'_, distinct_fragment_lanes::InputLanes>,
        apply_inner: ApplyInner<distinct_fragment_lanes::SnapshotLanes>,
    ) {
        self.0 += 1;
        apply_inner.apply_input_batch(snapshot, batch);
    }
}

contime::lanes! {
    mod fragment_lanes;
    snapshots [FragmentValueAt];
    routes [OnFragmentValueChanged => [FragmentValueAt]];
}

contime::lanes! {
    mod distinct_fragment_lanes;
    snapshots [AlphaAt, BetaAt];
    routes [OnAlphaChanged => [AlphaAt], OnBetaChanged => [BetaAt]];
}

contime::lanes! {
    mod merged_route_lanes;
    snapshots [SharedSourceAt, SharedMirrorAt];
    routes [
        OnSharedValueChanged => [SharedSourceAt],
        OnSharedValueChanged => [SharedMirrorAt],
    ];
}

fn snapshot_at<T, SL, IL>(contime: &contime::Contime<SL, IL>, time: i64, snapshot_id: u128) -> T
where
    SL: contime::SnapshotLanes<Time = i64, Input = IL> + 'static,
    IL: contime::InputLanes<SL> + 'static,
    T: TryFrom<SL>,
{
    contime
        .query_at(time, &[snapshot_id])
        .expect("snapshot query should succeed")
        .into_iter()
        .next()
        .expect("single snapshot lookup should return one slot")
        .and_then(|lane| T::try_from(lane).ok())
        .expect("snapshot should materialize")
}

#[test]
fn one_fragment_matches_the_one_shot_macro_behavior() {
    let legacy = legacy_lanes::Contime::new(1, 2_048);
    let fragmented = fragment_lanes::Contime::new(1, 2_048);
    legacy.apply([OnLegacyValueChanged { event_id: 10, time: 5, entity_id: 1, value: 7 }.into()]).unwrap();
    fragmented.apply([OnFragmentValueChanged { event_id: 20, time: 5, entity_id: 1, value: 7 }.into()]).unwrap();

    let legacy: LegacyValueAt = snapshot_at(&legacy, 6, 1);
    let fragmented: FragmentValueAt = snapshot_at(&fragmented, 6, 1);
    assert_eq!(legacy.value, fragmented.value, "composed and direct lane definitions produced different state");
}

#[test]
fn distinct_routes_merge_into_one_lane_universe() {
    let contime = distinct_fragment_lanes::Contime::new(1, 2_048);
    contime.apply([OnAlphaChanged { event_id: 11, time: 5, entity_id: 10_001, alpha: 3 }.into()]).unwrap();
    contime.apply([OnBetaChanged { event_id: 12, time: 5, entity_id: 20_001, beta: 9 }.into()]).unwrap();

    let alpha: AlphaAt = snapshot_at(&contime, 6, 10_001);
    let beta: BetaAt = snapshot_at(&contime, 6, 20_001);
    assert_eq!((alpha.alpha, beta.beta), (3, 9), "distinct routes did not retain their independent states");
}

#[test]
#[should_panic(expected = "different snapshot lane")]
fn one_snapshot_id_cannot_materialize_as_two_snapshot_types() {
    let (mut history, _) = contime::SnapshotHistory::<distinct_fragment_lanes::SnapshotLanes>::new_with_snapshot_id(50_000, 0, 1_000);

    history.apply_input_batch(
        vec![distinct_fragment_lanes::InputLanes::from(OnAlphaChanged { event_id: 21, time: 5, entity_id: 50_000, alpha: 3 })],
        &mut (),
    );
    history.apply_input_batch(
        vec![distinct_fragment_lanes::InputLanes::from(OnBetaChanged { event_id: 22, time: 6, entity_id: 50_000, beta: 9 })],
        &mut (),
    );
}

#[test]
fn incompatible_event_is_rejected_before_replay() {
    let (mut history, _) = contime::SnapshotHistory::<distinct_fragment_lanes::SnapshotLanes>::new_with_snapshot_id(50_000, 0, 1_000);
    let mut context = CountAppliedBatches::default();
    history.apply_input_batch(
        vec![distinct_fragment_lanes::InputLanes::from(OnAlphaChanged { event_id: 21, time: 5, entity_id: 50_000, alpha: 3 })],
        &mut context,
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        history.apply_input_batch(
            vec![distinct_fragment_lanes::InputLanes::from(OnBetaChanged { event_id: 22, time: 6, entity_id: 50_000, beta: 9 })],
            &mut context,
        );
    }));

    assert!(result.is_err(), "an incompatible event was accepted into a materialized snapshot history");
    assert_eq!(context.0, 1, "an incompatible event reached replay before the snapshot-lane invariant rejected it");
}

#[test]
fn repeated_route_keys_merge_targets_across_fragments() {
    let contime = merged_route_lanes::Contime::new(2, 2_048);
    contime.apply([OnSharedValueChanged { event_id: 13, time: 5, entity_id: 1, value: 21 }.into()]).unwrap();

    let source: SharedSourceAt = snapshot_at(&contime, 6, 30_001);
    let mirror: SharedMirrorAt = snapshot_at(&contime, 6, 40_001);
    assert_eq!((source.value, mirror.mirrored), (21, 21), "one event did not update every merged route target");

    let entries = contime.inspect_inputs(5..=5).unwrap();
    assert_eq!(entries.len(), 1, "one input was journaled once per route");
    assert_eq!(entries[0].routed_snapshot_ids, vec![30_001, 40_001], "journal lost or duplicated merged route targets");
}

#[test]
fn merged_event_materializes_each_foreign_target_identity() {
    let event = merged_route_lanes::InputLanes::from(OnSharedValueChanged { event_id: 14, time: 8, entity_id: 2, value: 34 });
    let snapshot_ids = <merged_route_lanes::InputLanes as contime::InputLanes<merged_route_lanes::SnapshotLanes>>::snapshot_ids(&event);

    assert_eq!(snapshot_ids.len(), 2, "merged event did not produce one route per target snapshot type");
    for snapshot_id in snapshot_ids {
        let snapshot = <merged_route_lanes::SnapshotLanes as contime::SnapshotLanes>::materialize(snapshot_id, &event)
            .expect("event should materialize its routed snapshot lane");
        assert!(matches!(contime::Snapshot::id(&snapshot), 30_002 | 40_002), "event route initialized the wrong target identity");
        assert_eq!(contime::Snapshot::time(&snapshot), 0, "identity initialization copied event state or time");
    }
}
