use contime::prelude::*;

struct Changed {
    time: i64,
    id: u64,
}
impl :: contime :: checkpoints :: Event for Changed { type Time = i64 ; fn time (& self) -> Self :: Time { self . time } }

impl contime::EventId for Changed {
    fn event_id(&self) -> u128 {
        1
    }
}

#[derive(Clone, Default)]

struct State {
    time: i64,
    id: u64,
}
impl :: contime :: checkpoints :: Snapshot for State { type Time = i64 ; fn time (& self) -> & Self :: Time { & self . time } fn set_time (& mut self , time : Self :: Time) { self . time = time ; } }


impl State {
    fn id_for(id: &u64) -> u128 {
        u128::from(*id)
    }
}
impl contime::SnapshotId for State {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.id)
    }
}
impl contime::SnapshotApply for State {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Changed>(|event, emit| {
            emit(<State>::id_for(&(event.id)));
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<Changed>() {
            let mut snapshot = Self::default();
            {
                snapshot.id = event.id;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {}
}

#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Changed], snapshots = [State])]
struct App;
fn start(config: ConTimeConfig<u64>) {
    let _ = ConTime::<App>::start(config);
}
fn main() {}
