use contime::{ContimeEvent, ContimeSnapshot};

#[derive(ContimeEvent)]
struct MissingEvent {time:i64,id:u128}

#[derive(ContimeSnapshot)]
#[contime_snapshot(events=[MissingEvent],apply={})]
struct MissingSnapshot {time:i64,id:u128}

fn main() {}
