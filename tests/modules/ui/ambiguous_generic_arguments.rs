use contime::prelude::*;



struct Added<T: std::hash::Hash> {
    time: i64,
    value: T,
}
impl < T : std :: hash :: Hash > :: contime :: checkpoints :: Event for Added < T > where i64 : :: core :: clone :: Clone + :: core :: cmp :: PartialEq { type Time = i64 ; fn time (& self) -> Self :: Time { self . time } }

impl<T: std::hash::Hash> contime::EventId for Added<T> {
    fn event_id(&self) -> u128 {
        1
    }
}

mod first {
    use super::*;
    type Value = i64;
    #[derive(ContimeModule)]
    #[contime_module(time = i64, events = [Added<Value>])]
    pub struct First;
}
mod second {
    use super::*;
    type Value = u64;
    #[derive(ContimeModule)]
    #[contime_module(time = i64, events = [Added<Value>])]
    pub struct Second;
}
#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [first::First, second::Second])]
struct App;
fn main() {}
