use contime::prelude::*;

struct Added {
    time: i64,
    id: u128,
}
impl :: contime :: checkpoints :: Event for Added { type Time = i64 ; fn time (& self) -> Self :: Time { self . time } }

impl EventId for Added {
    fn event_id(&self) -> u128 {
        self.id
    }
}

#[derive(Clone, Default)]

struct Total {
    time: i64,
    id: u128,
}
impl :: contime :: checkpoints :: Snapshot for Total { type Time = i64 ; fn time (& self) -> & Self :: Time { & self . time } fn set_time (& mut self , time : Self :: Time) { self . time = time ; } }

impl SnapshotId for Total {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}

struct OtherTime {
    time: u64,
    id: u128,
}
impl :: contime :: checkpoints :: Event for OtherTime { type Time = u64 ; fn time (& self) -> Self :: Time { self . time } }

impl EventId for OtherTime {
    fn event_id(&self) -> u128 {
        self.id
    }
}
impl SnapshotApply for Total {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<Added>(|e, emit| emit(e.id));
    }
    fn create(id: u128, _: EventRef<'_>) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for _ in batch.events::<OtherTime>() {}
    }
}
fn main() {}
