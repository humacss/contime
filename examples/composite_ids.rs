//! Composite identity fields mapped with Content ID 1.0.
//! Run: cargo run --example composite_ids --features macros
//! Source fields are explicit; hashing never hides which fields initialize state.
//! Compare with readme.rs for the same model using stored numeric IDs.
//! Type-name namespaces are local conventions: renaming a type changes its IDs.

use contime::prelude::*;
use std::{error::Error, io, time::Duration};

#[derive(ContimeEvent)]
#[contime(id = |v| (v.producer_id, v.sequence), map = event_id)]
struct Value {
    producer_id: u64,
    sequence: u64,
    owner_id: u64,
    list_id: u64,
    #[contime(time)]
    time: i64,
    value: char,
}
// Maps can be closures or ordinary functions; this example names them for reuse.
fn event_id(fields: (u64, u64)) -> u128 {
    contentid::typed_content_id_u128(std::any::type_name::<Value>(), &fields)
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| (v.owner_id, v.list_id), map = list_id)]
#[contime(events = [Value])]
struct List {
    owner_id: u64,
    list_id: u64,
    #[contime(time)]
    time: i64,
    values: Vec<char>,
}

fn list_id(fields: (u64, u64)) -> u128 {
    contentid::typed_content_id_u128(std::any::type_name::<List>(), &fields)
}

impl SnapshotApply for List {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for value in batch.events::<Value>() {
            self.values.push(value.value);
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
    let value = |sequence, time, value| Value { producer_id: 1, sequence, owner_id: 42, list_id: 7, time, value };
    // Repeating a source identity produces the same event ID.
    assert_eq!(value(1, 30, 'c').event_id(), value(1, 30, 'c').event_id());
    assert_ne!(value(1, 30, 'c').event_id(), value(2, -10, 'a').event_id());
    contime
        .apply([value(1, 30, 'c').into(), value(2, -10, 'a').into(), value(3, 10, 'b').into(), value(3, 10, 'b').into()])
        .map_err(api_error)?;
    contime.advance_to(30).map_err(api_error)?;
    contime.wait_until_idle(Duration::from_secs(5))?;
    assert!(contime.errors().try_iter().next().is_none());
    // The repeated event is ignored. Queries use exactly the same identity mapping as snapshots and routing.
    let snapshots = contime.query_at(30, [list_id((42, 7))]).map_err(api_error)?;
    assert_eq!(snapshots.len(), 1);
    let ListsSnapshotLanes::List(list) = &*snapshots[0];
    assert_eq!((list.owner_id, list.list_id), (42, 7));
    assert_eq!(list.snapshot_id(), list_id((42, 7)));
    assert_eq!(list.values, ['a', 'b', 'c']);
    println!("Verified composite Content IDs: {:?}", list.values);
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
