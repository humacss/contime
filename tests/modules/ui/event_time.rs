use contime::prelude::*;


struct Added {
    time: u64,
    counter_id: u128,
}
impl :: contime :: checkpoints :: Event for Added { type Time = u64 ; fn time (& self) -> Self :: Time { self . time } }

impl contime::EventId for Added {
    fn event_id(&self) -> u128 {
        1
    }
}

#[derive(Clone, Default)]

struct Counter {
    time: i64,
    counter_id: u128,
}
impl :: contime :: checkpoints :: Snapshot for Counter { type Time = i64 ; fn time (& self) -> & Self :: Time { & self . time } fn set_time (& mut self , time : Self :: Time) { self . time = time ; } }


impl Counter {
    fn id_for(counter_id: &u128) -> u128 {
        u128::from(*counter_id)
    }
}
impl contime::SnapshotId for Counter {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.counter_id)
    }
}
impl contime::SnapshotApply for Counter {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Added>(|event, emit| {
            emit(<Counter>::id_for(&(event.counter_id)));
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<Added>() {
            let mut snapshot = Self::default();
            {
                snapshot.counter_id = event.counter_id;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        for _ in batch.events::<Added>() {}
    }
}

#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added], snapshots = [Counter])]
struct App;

fn main() {}
