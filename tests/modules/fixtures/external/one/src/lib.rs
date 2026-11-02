use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};

#[derive(ContimeEvent)]
pub struct First {
    #[contime(time)]
    pub time: i64,
    pub id: u64,
}

impl contime::EventId for First {
    fn event_id(&self) -> u128 {
        100 + u128::from(self.id)
    }
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(event = First, id = first_state_ids)]
pub struct FirstState {
    #[contime(time)]
    pub time: i64,
    pub id: u64,
}

impl FirstState {
    pub fn id_for(id: &u64) -> u128 {
        1000 + u128::from(*id)
    }
}
impl contime::SnapshotId for FirstState {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.id)
    }
}
fn first_state_ids(event: &First, emit: &mut dyn FnMut(u128)) {
    emit(FirstState::id_for(&event.id));
}
impl contime::SnapshotCreate for FirstState {
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<First>() {
            let mut snapshot = Self::default();
            {
                snapshot.id = (event.id).clone();
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
}
impl contime::SnapshotApply for FirstState {
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        for _ in batch.events::<First>() {}
    }
}

#[derive(ContimeModule)]
#[contime(snapshots = [FirstState])]
pub struct Module;

#[derive(ContimeEvent)]
pub struct GenericInput<T, const N: usize = 4> {
    #[contime(id)]
    pub id: u128,
    pub owner: u128,
    #[contime(time)]
    pub at: i64,
    pub payload: T,
    pub marker: std::marker::PhantomData<[(); N]>,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.owner)]
#[contime(events = [GenericInput<T,N>])]
pub struct GenericState<T: Clone + Default + 'static, const N: usize = 4> {
    pub owner: u128,
    #[contime(time)]
    pub at: i64,
    pub payload: T,
    pub marker: std::marker::PhantomData<[(); N]>,
}
impl<T: Clone + Default + 'static, const N: usize> contime::SnapshotApply for GenericState<T, N> {}
#[derive(ContimeModule)]
#[contime(snapshots = [GenericState<Payload>])]
pub struct GenericModule;

// Deliberately use names that also occur inside generated metadata namespaces.
pub type Default0 = u64;
pub const DEFAULT_SIZE: usize = 8;
#[derive(ContimeEvent)]
pub struct Event0<T, const N: usize> {
    #[contime(id)]
    pub id: u128,
    pub owner: u128,
    #[contime(time)]
    pub at: i64,
    pub payload: T,
    pub marker: std::marker::PhantomData<[(); N]>,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id=|v| v.owner)]
#[contime(events=[Event0<T,N>])]
pub struct DefaultedState<T: Clone + Default + 'static = Default0, const N: usize = DEFAULT_SIZE> {
    pub owner: u128,
    #[contime(time)]
    pub at: i64,
    pub payload: T,
    pub marker: std::marker::PhantomData<[(); N]>,
}
impl<T: Clone + Default + 'static, const N: usize> contime::SnapshotApply for DefaultedState<T, N> {}

pub mod nested {
    use contime::{ContimeSnapshot, SnapshotApply};
    #[derive(Clone, Default, ContimeSnapshot)]
    #[contime(id=|v| v.id, map=|id| u128::from(id))]
    #[contime(events=[super::First])]
    pub struct RelativeState {
        pub id: u64,
        #[contime(time)]
        pub time: i64,
    }
    impl SnapshotApply for RelativeState {}
}

pub type Payload = u64;
pub trait Family {
    type Item: Clone + Default + 'static;
}
#[derive(Clone, Default)]
pub struct FamilyU64;
impl Family for FamilyU64 {
    type Item = u64;
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id=|v| v.owner)]
#[contime(events=[GenericInput<T::Item>])]
pub struct Projected<T: Family + Clone + Default + 'static>
where
    T::Item: 'static,
{
    pub owner: u128,
    #[contime(time)]
    pub at: i64,
    pub marker: std::marker::PhantomData<T>,
}
impl<T: Family + Clone + Default + 'static> contime::SnapshotApply for Projected<T> where T::Item: 'static {}
#[derive(ContimeModule)]
#[contime(snapshots=[Projected<FamilyU64>])]
pub struct ProjectedModule;

pub mod definitions {
    use contime::ContimeModule;
    pub type Payload = u32;
    #[derive(ContimeModule)]
    #[contime(snapshot=super::GenericState<super::Payload,{super::DEFAULT_SIZE}>,events=[super::GenericInput<super::Payload,{super::DEFAULT_SIZE}>])]
    pub struct Scoped;
}
