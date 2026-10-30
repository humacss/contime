use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};
#[derive(ContimeEvent)]
pub struct First {
    pub time: i64,
    pub id: u64,
}
#[derive(ContimeSnapshot)]
#[contime_snapshot(id=[self.id],events=[First],apply={for _ in batch.events{}})]
pub struct FirstState {
    pub time: i64,
    pub id: u64,
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[First],snapshots=[FirstState])]
pub struct Module;
