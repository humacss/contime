use contime::prelude::*;

struct WrongEvent {
    time: i64,
    raw_id: u64,
    owner: u64,
}
impl :: contime :: checkpoints :: Event for WrongEvent { type Time = i64 ; fn time (& self) -> Self :: Time { self . time } }

impl EventId for WrongEvent {
    fn event_id(&self) -> u128 {
        self.raw_id
    }
}
#[derive(Clone, Default)]

struct WrongSnapshot {
    time: i64,
    owner: u64,
}
impl :: contime :: checkpoints :: Snapshot for WrongSnapshot { type Time = i64 ; fn time (& self) -> & Self :: Time { & self . time } fn set_time (& mut self , time : Self :: Time) { self . time = time ; } }


impl WrongSnapshot {
    fn id_for(owner: &u64) -> u128 {
        *owner
    }
}
impl contime::SnapshotId for WrongSnapshot {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.owner)
    }
}
impl SnapshotApply for WrongSnapshot {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<WrongEvent>(|event, emit| {
            emit(<WrongSnapshot>::id_for(&(event.owner)));
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<WrongEvent>() {
            let mut snapshot = Self::default();
            {
                snapshot.owner = event.owner;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, _batch: impl contime::ApplyContext<Self>) {}
}
fn main() {}
