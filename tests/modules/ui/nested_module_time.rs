use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};

#[derive(ContimeEvent)]
struct Added {
    time: u64,
    counter_id: u128,
}

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = [self.counter_id], events = [Added], apply = {
    for event in batch.events { let CounterEvent::Added(_) = event; }
})]
struct Counter {
    time: u64,
    counter_id: u128,
}

#[derive(ContimeModule)]
#[contime_module(time = u64, events = [Added], snapshots = [Counter])]
struct Child;

#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [Child])]
struct App;

fn main() {}
