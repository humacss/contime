use contime::{ContimeModule };

pub struct First {
    pub time: i64,
    pub id: u64,
}
impl :: contime :: checkpoints :: Event for First { type Time = i64 ; fn time (& self) -> Self :: Time { self . time } }

impl contime::EventId for First {
    fn event_id(&self) -> u128 {
        100 + u128::from(self.id)
    }
}

#[derive(Clone, Default)]

pub struct FirstState {
    pub time: i64,
    pub id: u64,
}
impl :: contime :: checkpoints :: Snapshot for FirstState { type Time = i64 ; fn time (& self) -> & Self :: Time { & self . time } fn set_time (& mut self , time : Self :: Time) { self . time = time ; } }



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
impl contime :: SnapshotApply for FirstState { fn define (events : & mut contime :: EventSetBuilder < Self >) { events . event :: < First > (| event , emit | { emit (FirstState :: id_for (& event . id)) ; }) ; } fn create (snapshot_id : u128 , event : contime :: EventRef < '_ >) -> Self { if let Some (event) = event . get :: < First > () { let mut snapshot = Self :: default () ; { snapshot . id = (event . id) . clone () ; if contime :: SnapshotId :: snapshot_id (& snapshot) == snapshot_id { return snapshot ; } } } panic ! ("event does not initialize the requested snapshot") } fn apply_batch (& mut self , batch : impl contime :: ApplyContext < Self >) { for _ in batch . events :: < First > () { } } }

#[derive(ContimeModule)]
#[contime_module(time=i64,events=[First],snapshots=[FirstState])]
pub struct Module;
