use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};
#[derive(ContimeEvent)]
pub struct Second {
    pub time: i64,
    pub id: u64,
}
#[derive(ContimeSnapshot)]
#[contime_snapshot(id=[self.id],events=[Second],apply={for _ in batch.events{}})]
pub struct SecondState {
    pub time: i64,
    pub id: u64,
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Second],snapshots=[SecondState])]
pub struct Module;
