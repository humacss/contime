use contime::{ApplyEvent, Event, EventLanes as _, SeedSnapshot, Snapshot};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LegacyValueAt {
    entity_id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnLegacyValueChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LegacyValueAtEvent {
    OnLegacyValueChanged(OnLegacyValueChanged),
}

impl LegacyValueAt {
    fn lane_id(entity_id: u128) -> u128 {
        entity_id
    }
}

impl Snapshot for LegacyValueAt {
    type Event = LegacyValueAtEvent;

    fn id(&self) -> u128 {
        Self::lane_id(self.entity_id)
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        match event {
            LegacyValueAtEvent::OnLegacyValueChanged(event) => Self { entity_id: event.entity_id, time: event.time, value: event.value },
        }
    }
}

impl SeedSnapshot<OnLegacyValueChanged> for LegacyValueAt {
    fn seed_from_event(event: &OnLegacyValueChanged) -> Self {
        Self { entity_id: event.entity_id, time: event.time, value: event.value }
    }
}

impl Event for OnLegacyValueChanged {
    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl Event for LegacyValueAtEvent {
    fn id(&self) -> u128 {
        match self {
            Self::OnLegacyValueChanged(event) => event.id(),
        }
    }

    fn time(&self) -> i64 {
        match self {
            Self::OnLegacyValueChanged(event) => event.time(),
        }
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl ApplyEvent<LegacyValueAt> for OnLegacyValueChanged {
    fn snapshot_id(&self) -> u128 {
        LegacyValueAt::lane_id(self.entity_id)
    }

    fn apply_to(&self, snapshot: &mut LegacyValueAt) {
        snapshot.entity_id = self.entity_id;
        snapshot.time = self.time;
        snapshot.value = self.value;
    }
}

impl ApplyEvent<LegacyValueAt> for LegacyValueAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::OnLegacyValueChanged(event) => <OnLegacyValueChanged as ApplyEvent<LegacyValueAt>>::snapshot_id(event),
        }
    }

    fn apply_to(&self, snapshot: &mut LegacyValueAt) {
        match self {
            Self::OnLegacyValueChanged(event) => event.apply_to(snapshot),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FragmentValueAt {
    entity_id: u128,
    time: i64,
    value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnFragmentValueChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FragmentValueAtEvent {
    OnFragmentValueChanged(OnFragmentValueChanged),
}

impl FragmentValueAt {
    fn lane_id(entity_id: u128) -> u128 {
        1_000 + entity_id
    }
}

impl Snapshot for FragmentValueAt {
    type Event = FragmentValueAtEvent;

    fn id(&self) -> u128 {
        Self::lane_id(self.entity_id)
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        match event {
            FragmentValueAtEvent::OnFragmentValueChanged(event) => {
                Self { entity_id: event.entity_id, time: event.time, value: event.value }
            }
        }
    }
}

impl SeedSnapshot<OnFragmentValueChanged> for FragmentValueAt {
    fn seed_from_event(event: &OnFragmentValueChanged) -> Self {
        Self { entity_id: event.entity_id, time: event.time, value: event.value }
    }
}

impl Event for OnFragmentValueChanged {
    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl Event for FragmentValueAtEvent {
    fn id(&self) -> u128 {
        match self {
            Self::OnFragmentValueChanged(event) => event.id(),
        }
    }

    fn time(&self) -> i64 {
        match self {
            Self::OnFragmentValueChanged(event) => event.time(),
        }
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl ApplyEvent<FragmentValueAt> for OnFragmentValueChanged {
    fn snapshot_id(&self) -> u128 {
        FragmentValueAt::lane_id(self.entity_id)
    }

    fn apply_to(&self, snapshot: &mut FragmentValueAt) {
        snapshot.entity_id = self.entity_id;
        snapshot.time = self.time;
        snapshot.value = self.value;
    }
}

impl ApplyEvent<FragmentValueAt> for FragmentValueAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::OnFragmentValueChanged(event) => <OnFragmentValueChanged as ApplyEvent<FragmentValueAt>>::snapshot_id(event),
        }
    }

    fn apply_to(&self, snapshot: &mut FragmentValueAt) {
        match self {
            Self::OnFragmentValueChanged(event) => event.apply_to(snapshot),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AlphaAt {
    entity_id: u128,
    time: i64,
    alpha: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnAlphaChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    alpha: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AlphaAtEvent {
    OnAlphaChanged(OnAlphaChanged),
}

impl AlphaAt {
    fn lane_id(entity_id: u128) -> u128 {
        10_000 + entity_id
    }
}

impl Snapshot for AlphaAt {
    type Event = AlphaAtEvent;

    fn id(&self) -> u128 {
        Self::lane_id(self.entity_id)
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        match event {
            AlphaAtEvent::OnAlphaChanged(event) => Self { entity_id: event.entity_id, time: event.time, alpha: event.alpha },
        }
    }
}

impl SeedSnapshot<OnAlphaChanged> for AlphaAt {
    fn seed_from_event(event: &OnAlphaChanged) -> Self {
        Self { entity_id: event.entity_id, time: event.time, alpha: event.alpha }
    }
}

impl Event for OnAlphaChanged {
    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl Event for AlphaAtEvent {
    fn id(&self) -> u128 {
        match self {
            Self::OnAlphaChanged(event) => event.id(),
        }
    }

    fn time(&self) -> i64 {
        match self {
            Self::OnAlphaChanged(event) => event.time(),
        }
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl ApplyEvent<AlphaAt> for OnAlphaChanged {
    fn snapshot_id(&self) -> u128 {
        AlphaAt::lane_id(self.entity_id)
    }

    fn apply_to(&self, snapshot: &mut AlphaAt) {
        snapshot.entity_id = self.entity_id;
        snapshot.time = self.time;
        snapshot.alpha = self.alpha;
    }
}

impl ApplyEvent<AlphaAt> for AlphaAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::OnAlphaChanged(event) => <OnAlphaChanged as ApplyEvent<AlphaAt>>::snapshot_id(event),
        }
    }

    fn apply_to(&self, snapshot: &mut AlphaAt) {
        match self {
            Self::OnAlphaChanged(event) => event.apply_to(snapshot),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BetaAt {
    entity_id: u128,
    time: i64,
    beta: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnBetaChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    beta: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BetaAtEvent {
    OnBetaChanged(OnBetaChanged),
}

impl BetaAt {
    fn lane_id(entity_id: u128) -> u128 {
        20_000 + entity_id
    }
}

impl Snapshot for BetaAt {
    type Event = BetaAtEvent;

    fn id(&self) -> u128 {
        Self::lane_id(self.entity_id)
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        match event {
            BetaAtEvent::OnBetaChanged(event) => Self { entity_id: event.entity_id, time: event.time, beta: event.beta },
        }
    }
}

impl SeedSnapshot<OnBetaChanged> for BetaAt {
    fn seed_from_event(event: &OnBetaChanged) -> Self {
        Self { entity_id: event.entity_id, time: event.time, beta: event.beta }
    }
}

impl Event for OnBetaChanged {
    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl Event for BetaAtEvent {
    fn id(&self) -> u128 {
        match self {
            Self::OnBetaChanged(event) => event.id(),
        }
    }

    fn time(&self) -> i64 {
        match self {
            Self::OnBetaChanged(event) => event.time(),
        }
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl ApplyEvent<BetaAt> for OnBetaChanged {
    fn snapshot_id(&self) -> u128 {
        BetaAt::lane_id(self.entity_id)
    }

    fn apply_to(&self, snapshot: &mut BetaAt) {
        snapshot.entity_id = self.entity_id;
        snapshot.time = self.time;
        snapshot.beta = self.beta;
    }
}

impl ApplyEvent<BetaAt> for BetaAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::OnBetaChanged(event) => <OnBetaChanged as ApplyEvent<BetaAt>>::snapshot_id(event),
        }
    }

    fn apply_to(&self, snapshot: &mut BetaAt) {
        match self {
            Self::OnBetaChanged(event) => event.apply_to(snapshot),
        }
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct OnSharedValueChanged {
    event_id: u128,
    time: i64,
    entity_id: u128,
    value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SharedSourceAtEvent {
    OnSharedValueChanged(OnSharedValueChanged),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SharedMirrorAtEvent {
    OnSharedValueChanged(OnSharedValueChanged),
}

impl SharedSourceAt {
    fn lane_id(entity_id: u128) -> u128 {
        30_000 + entity_id
    }
}

impl SharedMirrorAt {
    fn lane_id(entity_id: u128) -> u128 {
        40_000 + entity_id
    }
}

impl Snapshot for SharedSourceAt {
    type Event = SharedSourceAtEvent;

    fn id(&self) -> u128 {
        Self::lane_id(self.entity_id)
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        match event {
            SharedSourceAtEvent::OnSharedValueChanged(event) => Self { entity_id: event.entity_id, time: event.time, value: event.value },
        }
    }
}

impl SeedSnapshot<OnSharedValueChanged> for SharedSourceAt {
    fn seed_from_event(event: &OnSharedValueChanged) -> Self {
        Self { entity_id: event.entity_id, time: event.time, value: event.value }
    }
}

impl Snapshot for SharedMirrorAt {
    type Event = SharedMirrorAtEvent;

    fn id(&self) -> u128 {
        Self::lane_id(self.entity_id)
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }

    fn from_event(event: &Self::Event) -> Self {
        match event {
            SharedMirrorAtEvent::OnSharedValueChanged(event) => {
                Self { entity_id: event.entity_id, time: event.time, mirrored: event.value }
            }
        }
    }
}

impl SeedSnapshot<OnSharedValueChanged> for SharedMirrorAt {
    fn seed_from_event(event: &OnSharedValueChanged) -> Self {
        Self { entity_id: event.entity_id, time: event.time, mirrored: event.value }
    }
}

impl Event for OnSharedValueChanged {
    fn id(&self) -> u128 {
        self.event_id
    }

    fn time(&self) -> i64 {
        self.time
    }

    fn conservative_size(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
    }
}

impl Event for SharedSourceAtEvent {
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
        std::mem::size_of::<Self>() as u64
    }
}

impl Event for SharedMirrorAtEvent {
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
        std::mem::size_of::<Self>() as u64
    }
}

impl ApplyEvent<SharedSourceAt> for OnSharedValueChanged {
    fn snapshot_id(&self) -> u128 {
        SharedSourceAt::lane_id(self.entity_id)
    }

    fn apply_to(&self, snapshot: &mut SharedSourceAt) {
        snapshot.entity_id = self.entity_id;
        snapshot.time = self.time;
        snapshot.value = self.value;
    }
}

impl ApplyEvent<SharedMirrorAt> for OnSharedValueChanged {
    fn snapshot_id(&self) -> u128 {
        SharedMirrorAt::lane_id(self.entity_id)
    }

    fn apply_to(&self, snapshot: &mut SharedMirrorAt) {
        snapshot.entity_id = self.entity_id;
        snapshot.time = self.time;
        snapshot.mirrored = self.value;
    }
}

impl ApplyEvent<SharedSourceAt> for SharedSourceAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::OnSharedValueChanged(event) => <OnSharedValueChanged as ApplyEvent<SharedSourceAt>>::snapshot_id(event),
        }
    }

    fn apply_to(&self, snapshot: &mut SharedSourceAt) {
        match self {
            Self::OnSharedValueChanged(event) => event.apply_to(snapshot),
        }
    }
}

impl ApplyEvent<SharedMirrorAt> for SharedMirrorAtEvent {
    fn snapshot_id(&self) -> u128 {
        match self {
            Self::OnSharedValueChanged(event) => <OnSharedValueChanged as ApplyEvent<SharedMirrorAt>>::snapshot_id(event),
        }
    }

    fn apply_to(&self, snapshot: &mut SharedMirrorAt) {
        match self {
            Self::OnSharedValueChanged(event) => event.apply_to(snapshot),
        }
    }
}

contime::fragment! {
    macro fragment_value_fragment;

    snapshots {
        FragmentValueAt,
    }

    routes {
        OnFragmentValueChanged(OnFragmentValueChanged) => [FragmentValueAt],
    }
}

contime::fragment! {
    macro alpha_fragment;

    snapshots {
        AlphaAt,
    }

    routes {
        OnAlphaChanged(OnAlphaChanged) => [AlphaAt],
    }
}

contime::fragment! {
    macro beta_fragment;

    snapshots {
        BetaAt,
    }

    routes {
        OnBetaChanged(OnBetaChanged) => [BetaAt],
    }
}

contime::fragment! {
    macro shared_primary_fragment;

    snapshots {
        SharedSourceAt,
        SharedMirrorAt,
    }

    routes {
        OnSharedValueChanged(OnSharedValueChanged) => [SharedSourceAt],
    }
}

contime::fragment! {
    macro shared_secondary_fragment;

    snapshots {
        SharedSourceAt,
        SharedMirrorAt,
    }

    routes {
        OnSharedValueChanged(OnSharedValueChanged) => [SharedMirrorAt],
    }
}

contime::contime! {
    mod legacy_lanes;
    snapshots { LegacyValueAt },
    OnLegacyValueChanged(OnLegacyValueChanged) => [LegacyValueAt],
}

contime::lanes! {
    mod fragment_lanes;
    fragments [
        fragment_value_fragment,
    ];
}

contime::lanes! {
    mod distinct_fragment_lanes;
    fragments [
        alpha_fragment,
        beta_fragment,
    ];
}

contime::lanes! {
    mod merged_route_lanes;
    fragments [
        shared_primary_fragment,
        shared_secondary_fragment,
    ];
}

fn snapshot_at<T, SL, EL>(contime: &contime::Contime<SL, EL>, time: i64, snapshot_id: u128) -> T
where
    SL: contime::SnapshotLanes<Event = EL> + 'static,
    EL: contime::EventLanes<SL> + 'static,
    T: TryFrom<SL>,
{
    contime
        .many_at(time, &[snapshot_id])
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

    legacy.apply_event(OnLegacyValueChanged { event_id: 10, time: 5, entity_id: 1, value: 7 }).expect("legacy event should apply");
    fragmented.apply_event(OnFragmentValueChanged { event_id: 20, time: 5, entity_id: 1, value: 7 }).expect("fragment event should apply");

    let legacy_snapshot = snapshot_at::<LegacyValueAt, _, _>(&legacy, 6, LegacyValueAt::lane_id(1));
    let fragmented_snapshot = snapshot_at::<FragmentValueAt, _, _>(&fragmented, 6, FragmentValueAt::lane_id(1));

    assert_eq!(legacy_snapshot.value, fragmented_snapshot.value);
}

#[test]
fn distinct_routes_merge_into_one_lane_universe() {
    let contime = distinct_fragment_lanes::Contime::new(1, 2_048);

    contime.apply_event(OnAlphaChanged { event_id: 11, time: 5, entity_id: 1, alpha: 3 }).expect("alpha event should apply");
    contime.apply_event(OnBetaChanged { event_id: 12, time: 5, entity_id: 1, beta: 9 }).expect("beta event should apply");

    let alpha = snapshot_at::<AlphaAt, _, _>(&contime, 6, AlphaAt::lane_id(1));
    let beta = snapshot_at::<BetaAt, _, _>(&contime, 6, BetaAt::lane_id(1));

    assert_eq!(alpha.alpha, 3);
    assert_eq!(beta.beta, 9);
}

#[test]
fn repeated_route_keys_merge_targets_across_fragments() {
    let contime = merged_route_lanes::Contime::new(1, 2_048);

    contime.apply_event(OnSharedValueChanged { event_id: 13, time: 5, entity_id: 1, value: 21 }).expect("shared event should apply");

    let source = snapshot_at::<SharedSourceAt, _, _>(&contime, 6, SharedSourceAt::lane_id(1));
    let mirror = snapshot_at::<SharedMirrorAt, _, _>(&contime, 6, SharedMirrorAt::lane_id(1));

    assert_eq!(source.value, 21);
    assert_eq!(mirror.mirrored, 21);
}

#[test]
fn merged_event_snapshots_materialize_all_foreign_targets_without_wrapper_conversions() {
    let event = merged_route_lanes::EventLanes::from(OnSharedValueChanged { event_id: 14, time: 8, entity_id: 2, value: 34 });

    let snapshots = event.snapshots();
    assert_eq!(snapshots.len(), 2);

    let source = snapshots
        .iter()
        .find_map(|lane| match lane {
            merged_route_lanes::SnapshotLanes::SharedSourceAt(snapshot) => Some(snapshot.clone()),
            _ => None,
        })
        .expect("source snapshot should materialize");
    let mirror = snapshots
        .iter()
        .find_map(|lane| match lane {
            merged_route_lanes::SnapshotLanes::SharedMirrorAt(snapshot) => Some(snapshot.clone()),
            _ => None,
        })
        .expect("mirror snapshot should materialize");

    assert_eq!(source.value, 34);
    assert_eq!(mirror.mirrored, 34);
    assert_eq!(source.time, 8);
    assert_eq!(mirror.time, 8);
}
