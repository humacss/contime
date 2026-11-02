//! Compose dependent modules into one ConTime instance.
//!
//! Run with `cargo run --example modules --features macros`.
//! Start with `readme.rs` for the smaller step-by-step walkthrough.
//!
//! Lists registers the ordered list. Statistics depends on Lists and adds a
//! count snapshot; App includes both modules. Their shared Lists dependency is
//! registered once. Each input updates the list and its statistics, whose snapshot
//! IDs remain distinct even though they share the same logical list_id.
//!
//! The first query observes a, c. A late b changes the list to a, b, c and the
//! count from two to three. Assertions check both snapshots after each insertion.
//! Application time uses logical milliseconds, including negative timestamps.

use contime::prelude::*;
use std::{error::Error, io, time::Duration};

#[derive(ContimeEvent)]
struct Value {
    #[contime(id)]
    event_id: u128,
    #[contime(time)]
    time: i64,
    list_id: u128,
    value: char,
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |value| value.list_id, map = list_key)]
#[contime(events = [Value])]
struct List {
    #[contime(time)]
    time: i64,
    list_id: u128,
    values: Vec<(i64, char)>,
}

// Distinct ID ranges keep the two snapshot types separate in this small demo.
// Production applications should choose an identity scheme for their full domain.
fn list_key(list_id: u128) -> u128 {
    1000 + list_id
}

impl SnapshotApply for List {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Value>() {
            self.values.push((event.time, event.value));
        }
    }
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |value| value.list_id, map = statistics_key)]
#[contime(events = [Value])]
struct ValueStatistics {
    #[contime(time)]
    time: i64,
    list_id: u128,
    count: usize,
}

fn statistics_key(list_id: u128) -> u128 {
    2000 + list_id
}

impl SnapshotApply for ValueStatistics {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for _ in batch.events::<Value>() {
            self.count += 1;
        }
    }
}

#[derive(ContimeModule)]
#[contime(snapshots = [List])]
#[allow(dead_code)]
struct Lists;

#[derive(ContimeModule)]
#[contime(snapshots = [ValueStatistics])]
#[contime(modules = [Lists])]
#[allow(dead_code)]
struct Statistics;

// Lists is reachable twice. Each event and snapshot lane must occur once.
#[derive(ContimeModule)]
#[contime(modules = [Lists, Statistics])]
struct App;

fn main() -> Result<(), Box<dyn Error>> {
    // Integer timestamps are logical application units, here milliseconds.
    // Negative times can represent inputs preceding the chosen origin.
    let config = ConTimeConfig::<i64> {
        router_count: 1,
        worker_count: 2,
        placement: contime::Placement::default(),
        pruning_interval: Duration::from_secs(60),
        history_retention: 60_000,
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 32 },
    };
    let contime = ConTime::<App>::start(config)?;
    let result = demonstrate(&contime);
    let stopped = shutdown(contime);
    result?;
    stopped?;
    Ok(())
}

fn demonstrate(contime: &ConTime<App>) -> Result<(), Box<dyn Error>> {
    contime
        .apply([
            Value { event_id: 1, time: -10, list_id: 7, value: 'a' }.into(),
            Value { event_id: 2, time: 30, list_id: 7, value: 'c' }.into(),
        ])
        .map_err(api_error)?;
    assert_snapshots(contime, &[(-10, 'a'), (30, 'c')])?;

    // A late input updates both snapshot types through the same event lane.
    contime.apply([Value { event_id: 3, time: 10, list_id: 7, value: 'b' }.into()]).map_err(api_error)?;
    assert_snapshots(contime, &[(-10, 'a'), (10, 'b'), (30, 'c')])?;
    println!("Verified shared module dependencies: one list and one statistics snapshot");
    Ok(())
}

fn assert_snapshots(contime: &ConTime<App>, expected: &[(i64, char)]) -> Result<(), Box<dyn Error>> {
    contime.advance_to(30).map_err(api_error)?;
    // This finite demonstration pauses submissions; live applications should
    // synchronize individual requests instead of waiting for global idle.
    contime.wait_until_idle(Duration::from_secs(5))?;
    assert!(contime.errors().try_iter().next().is_none());
    let snapshots = contime.query_at(30, [list_key(7), statistics_key(7)]).map_err(api_error)?;
    assert_eq!(snapshots.len(), 2);
    let mut saw_list = false;
    let mut saw_statistics = false;
    for snapshot in snapshots {
        match *snapshot {
            AppSnapshotLanes::List(state) => {
                assert!(!saw_list);
                saw_list = true;
                assert_eq!(state.list_id, 7);
                assert_eq!(state.values, expected);
            }
            AppSnapshotLanes::ValueStatistics(state) => {
                assert!(!saw_statistics);
                saw_statistics = true;
                assert_eq!(state.list_id, 7);
                assert_eq!(state.count, expected.len());
            }
        }
    }
    assert!(saw_list && saw_statistics);
    Ok(())
}

// Join the owned background threads on either success or an API error.
fn shutdown(contime: ConTime<App>) -> io::Result<()> {
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
