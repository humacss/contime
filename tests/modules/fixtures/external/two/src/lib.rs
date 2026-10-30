use contime::{ContimeModule };

pub struct Second {
    pub time: i64,
    pub id: u64,
}
impl :: contime :: checkpoints :: Event for Second { type Time = i64 ; fn time (& self) -> Self :: Time { self . time } }

impl contime::EventId for Second {
    fn event_id(&self) -> u128 {
        200 + u128::from(self.id)
    }
}

#[derive(Clone, Default)]

pub struct SecondState {
    pub time: i64,
    pub id: u64,
}
impl :: contime :: checkpoints :: Snapshot for SecondState { type Time = i64 ; fn time (& self) -> & Self :: Time { & self . time } fn set_time (& mut self , time : Self :: Time) { self . time = time ; } }



impl SecondState {
    pub fn id_for(id: &u64) -> u128 {
        2000 + u128::from(*id)
    }
}
impl contime::SnapshotId for SecondState {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.id)
    }
}
impl contime :: SnapshotApply for SecondState { fn define (events : & mut contime :: EventSetBuilder < Self >) { events . event :: < Second > (| event , emit | { emit (SecondState :: id_for (& event . id)) ; }) ; } fn create (snapshot_id : u128 , event : contime :: EventRef < '_ >) -> Self { if let Some (event) = event . get :: < Second > () { let mut snapshot = Self :: default () ; { snapshot . id = (event . id) . clone () ; if contime :: SnapshotId :: snapshot_id (& snapshot) == snapshot_id { return snapshot ; } } } panic ! ("event does not initialize the requested snapshot") } fn apply_batch (& mut self , batch : impl contime :: ApplyContext < Self >) { for _ in batch . events :: < Second > () { } } }

#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Second],snapshots=[SecondState])]
pub struct Module;
