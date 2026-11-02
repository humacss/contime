//! Module-local subsets with nested module composition and inferred time/events.
//! Run: cargo run --example module_subsets --features macros
//! Start with modules.rs for composition; this example adds event filtering.
//! Both snapshots support Clear. Only Audit receives it in this module; Clear
//! remains a module event because Audit imports it through its default settings.

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

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.list_id, map = |id| id + 1000)]
#[contime(events = [Value, Clear])]
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
    }
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.list_id, map = |id| id + 2000)]
#[contime(events = [Value, Clear])]
struct Audit {
    list_id: u128,
    #[contime(time)]
    time: i64,
    values: Vec<char>,
}

impl SnapshotApply for Audit {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for _ in batch.events::<Clear>() {
            self.values.clear();
        }
        for event in batch.events::<Value>() {
            self.values.push(event.value);
        }
    }
}

#[derive(ContimeModule)]
#[contime(snapshots = [Audit])]
#[allow(dead_code)]
struct Auditing;

#[derive(ContimeModule)]
#[contime(snapshot = List, events = [Value])]
#[contime(modules = [Auditing])]
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
            Value { event_id: 3, list_id: 7, time: 10, value: 'b' }.into(),
            Clear { event_id: 4, list_id: 7, time: 40 }.into(),
        ])
        .map_err(api_error)?;
    contime.advance_to(40).map_err(api_error)?;
    contime.wait_until_idle(Duration::from_secs(5))?;
    assert!(contime.errors().try_iter().next().is_none());
    let snapshots = contime.query_at(40, [1007, 2007]).map_err(api_error)?;
    assert_eq!(snapshots.len(), 2);
    for state in snapshots {
        match *state {
            ListsSnapshotLanes::List(list) => {
                assert_eq!(list.list_id, 7);
                assert_eq!(list.values, ['a', 'b', 'c']);
            }
            ListsSnapshotLanes::Audit(audit) => {
                assert_eq!(audit.list_id, 7);
                assert!(audit.values.is_empty());
            }
        }
    }
    println!("Verified module-local event subsets and nested composition");
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
