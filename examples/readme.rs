//! The README walkthrough: one event, one snapshot, and late insertion.
//! Run: cargo run --example readme --features macros
//! Definitions come first; configuration and shutdown helpers are at the bottom.

use contime::prelude::*;
use std::{error::Error, io, time::Duration};

// This ID uniquely identifies the event; duplicate submissions are ignored.
#[derive(ContimeEvent)]
struct Value {
    #[contime(id)]
    event_id: u128,
    list_id: u128,
    #[contime(time)]
    time: i64,
    value: char,
}

// Register Value events. Their list_id selects the snapshot
// with the same list_id.
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(events = [Value])]
struct List {
    #[contime(id)]
    list_id: u128,
    #[contime(time)]
    time: i64,
    values: Vec<char>,
}

// Append each value to its list.
impl SnapshotApply for List {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Value>() {
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
    // Start with c. The next insertions arrive later but belong before it.
    contime.apply([Value { event_id: 1, list_id: 7, time: 30, value: 'c' }.into()]).map_err(api_error)?;
    assert_eq!(query_list(contime)?, ['c']);

    // Negative timestamps are ordinary times before the chosen origin.
    contime.apply([Value { event_id: 2, list_id: 7, time: -10, value: 'a' }.into()]).map_err(api_error)?;
    assert_eq!(query_list(contime)?, ['a', 'c']);

    contime.apply([Value { event_id: 3, list_id: 7, time: 10, value: 'b' }.into()]).map_err(api_error)?;
    assert_eq!(query_list(contime)?, ['a', 'b', 'c']);

    // A retry with the same event ID must not append b again.
    contime.apply([Value { event_id: 3, list_id: 7, time: 10, value: 'b' }.into()]).map_err(api_error)?;
    let values = query_list(contime)?;
    assert_eq!(values, ['a', 'b', 'c']);
    println!("Verified out-of-order insertion and retry: {values:?}");
    Ok(())
}

fn query_list(contime: &ConTime<Lists>) -> Result<Vec<char>, Box<dyn Error>> {
    contime.advance_to(30).map_err(api_error)?;
    // This demonstration stops submissions before each wait. Global idle waits
    // are helpers for tests and benchmarks, not synchronization for live traffic.
    contime.wait_until_idle(Duration::from_secs(5))?;
    assert!(contime.errors().try_iter().next().is_none(), "every insertion must be accepted");
    let snapshots = contime.query_at(30, [7]).map_err(api_error)?;
    assert_eq!(snapshots.len(), 1);
    let ListsSnapshotLanes::List(list) = &*snapshots[0];
    assert_eq!(list.list_id, 7);
    Ok(list.values.clone())
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
