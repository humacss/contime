use contime::{ContimeEvent, ContimeSnapshot};

#[derive(ContimeEvent)]
#[contime_event(id = |event| event.raw_id)]
struct WrongEvent {time:i64,raw_id:u64,owner:u64}

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |owner| *owner, events=[WrongEvent],apply={})]
struct WrongSnapshot {time:i64,owner:u64}

fn main() {}
