//! Default event lists, renamed source fields, and per-event ID mapping.
//! Run: cargo run --example event_registration --features macros
//! Unlike readme.rs, ForwardedValue uses destination rather than list_id.
//! Selectors must have matching shape/types; mappings must identify the created state.

use contime::prelude::*;
use std::{error::Error, io, time::Duration};

#[derive(ContimeEvent)]
struct Value {
    #[contime(id)]
    event_id: u128,
    list_id: u128,
    #[contime(time)]
    time: i64,
    value: char,
}

#[derive(ContimeEvent)]
struct Clear {
    #[contime(id)]
    event_id: u128,
    list_id: u128,
    #[contime(time)]
    time: i64,
}

#[derive(ContimeEvent)]
struct ForwardedValue {
    #[contime(id)]
    event_id: u128,
    destination: u128,
    #[contime(time)]
    time: i64,
    value: char,
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.list_id, map = |id| id + 1000)]
#[contime(events = [Value, Clear])]
#[contime(event = ForwardedValue, id = |v| v.destination, map = |id| 1000 + id)]
struct List {
    list_id: u128,
    #[contime(time)]
    time: i64,
    values: Vec<char>,
}

impl SnapshotApply for List {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        // Each type gets its own phase, in an order chosen by the application.
        for _ in batch.events::<Clear>() {
            self.values.clear();
        }
        for event in batch.events::<Value>() {
            self.values.push(event.value);
        }
        for event in batch.events::<ForwardedValue>() {
            self.values.push(event.value);
        }
    }
}

#[derive(ContimeModule)]
#[contime(snapshots = [List])]
struct Lists;

fn main() -> Result<(), Box<dyn Error>> {
    let contime = ConTime::<Lists>::start(configuration())?;
    let result = demonstrate(&contime);
    let stopped = shutdown(contime);
    result?;
    stopped?;
    Ok(())
}

fn demonstrate(contime: &ConTime<Lists>) -> Result<(), Box<dyn Error>> {
    contime
        .apply([
            Value { event_id: 1, list_id: 7, time: 30, value: 'c' }.into(),
            Value { event_id: 2, list_id: 7, time: -10, value: 'a' }.into(),
            ForwardedValue { event_id: 3, destination: 7, time: 10, value: 'b' }.into(),
            Clear { event_id: 4, list_id: 7, time: 40 }.into(),
        ])
        .map_err(api_error)?;
    contime.advance_to(40).map_err(api_error)?;
    contime.wait_until_idle(Duration::from_secs(5))?;
    assert!(contime.errors().try_iter().next().is_none());
    // The same mapper turns the logical list ID 7 into history ID 1007.
    for (time, expected) in [(30, vec!['a', 'b', 'c']), (40, vec![])] {
        let snapshots = contime.query_at(time, [1007]).map_err(api_error)?;
        assert_eq!(snapshots.len(), 1);
        let ListsSnapshotLanes::List(list) = &*snapshots[0];
        assert_eq!(list.list_id, 7);
        assert_eq!(list.snapshot_id(), 1007);
        assert_eq!(list.values, expected);
    }
    println!("Verified inherited registration and renamed identity fields");
    Ok(())
}

fn configuration() -> ConTimeConfig<i64> {
    ConTimeConfig {
        router_count: 1,
        worker_count: 2,
        placement: contime::Placement::default(),
        pruning_interval: Duration::from_secs(60),
        history_retention: 60_000,
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 32 },
    }
}

// main calls this even if an API request fails, so background threads are joined.
fn shutdown(contime: ConTime<Lists>) -> io::Result<()> {
    let report = contime.shutdown();
    if report.routers.iter().any(|outcome| *outcome != contime::ThreadOutcome::Completed)
        || report.workers.iter().any(|outcome| *outcome != contime::ThreadOutcome::Completed)
    {
        return Err(io::Error::other("a ConTime thread did not complete normally"));
    }
    Ok(())
}

fn api_error(error: contime::ApiError) -> io::Error {
    io::Error::other(format!("ConTime request failed: {error:?}"))
}
