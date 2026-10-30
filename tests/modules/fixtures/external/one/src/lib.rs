use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};
#[derive(ContimeEvent)]
#[contime_event(id = |event| 100 + u128::from(event.id))]
pub struct First {
    pub time: i64,
    pub id: u64,
}
#[derive(ContimeSnapshot)]
#[contime_snapshot(id=|id| 1000 + u128::from(*id),events=[First],apply={for _ in batch.events{}})]
pub struct FirstState {
    pub time: i64,
    pub id: u64,
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[First],snapshots=[FirstState])]
pub struct Module;
