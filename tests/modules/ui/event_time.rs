use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};

#[derive(ContimeEvent)]
#[contime_event(id = |_event| 1)]
struct Added {
    time: u64,
    counter_id: u128,
}

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |counter_id| u128::from(*counter_id), events = [Added], apply = {
    for event in batch.events { let CounterEvent::Added(_) = event; }
})]
struct Counter {
    time: i64,
    counter_id: u128,
}

#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added], snapshots = [Counter])]
struct App;

fn main() {}
