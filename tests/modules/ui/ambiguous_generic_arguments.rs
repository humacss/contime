use contime::{ContimeEvent, ContimeModule};

#[derive(ContimeEvent)]
#[contime_event(id = |_event| 1)]
struct Added<T: std::hash::Hash> {
    time: i64,
    value: T,
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
