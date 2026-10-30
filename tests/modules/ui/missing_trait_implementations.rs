use contime::prelude::*;

struct MissingEvent {
    time: i64,
    id: u128,
}
impl :: contime :: checkpoints :: Event for MissingEvent { type Time = i64 ; fn time (& self) -> Self :: Time { self . time } }

#[derive(Clone, Default)]
struct MissingSnapshot {
    time: i64,
    id: u128,
}
impl :: contime :: checkpoints :: Snapshot for MissingSnapshot { type Time = i64 ; fn time (& self) -> & Self :: Time { & self . time } fn set_time (& mut self , time : Self :: Time) { self . time = time ; } }

impl SnapshotApply for MissingSnapshot {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<MissingEvent>(|e, emit| emit(e.id));
    }
    fn create(id: u128, _: EventRef<'_>) -> Self {
        Self { id, ..Self::default() }
    }
    fn apply_batch(&mut self, _: impl ApplyContext<Self>) {}
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[MissingEvent],snapshots=[MissingSnapshot])]
struct App;
fn main() {}
