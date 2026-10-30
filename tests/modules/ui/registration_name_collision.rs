use contime::prelude::*;
struct First;
struct Second;
impl contime::checkpoints::Event for First { type Time=i64;fn time(&self)->i64{0} }
impl EventId for First {fn event_id(&self)->u128{1}}
impl contime::checkpoints::Event for Second { type Time=i64;fn time(&self)->i64{0} }
impl EventId for Second {fn event_id(&self)->u128{2}}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Same=First,Same=Second])]
struct App;
fn main() {}
