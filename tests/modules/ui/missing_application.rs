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
struct State {
    time: i64,
    id: u128,
}
impl :: contime :: checkpoints :: Snapshot for State { type Time = i64 ; fn time (& self) -> & Self :: Time { & self . time } fn set_time (& mut self , time : Self :: Time) { self . time = time ; } }

impl SnapshotId for State {
    fn snapshot_id(&self) -> u128 {
        self.id
    }
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added],snapshots=[State])]
struct App;
fn main() {}
