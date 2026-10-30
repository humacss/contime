use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};
#[derive(ContimeEvent)]
#[contime_event(id = |event| 200 + u128::from(event.id))]
pub struct Second {
    pub time: i64,
    pub id: u64,
}
#[derive(ContimeSnapshot)]
#[contime_snapshot(id=|id| 2000 + u128::from(*id),events=[Second],apply={for _ in batch.events{}})]
pub struct SecondState {
    pub time: i64,
    pub id: u64,
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Second],snapshots=[SecondState])]
pub struct Module;
