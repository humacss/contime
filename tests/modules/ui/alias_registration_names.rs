use contime::prelude::*;
struct Added;
impl contime::checkpoints::Event for Added {
    type Time = i64;
    fn time(&self) -> i64 { 0 }
}
impl EventId for Added { fn event_id(&self) -> u128 { 1 } }
use Added as Incoming;
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Added,Incoming])]
struct App;
fn main() {}
