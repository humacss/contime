//! A finite, assertion-based walkthrough of out-of-order event application.
//!
//! Run with `cargo run --example ordered_list --features macros`.
//! Each insertion is followed by a replay wait and a query, so every intermediate
//! result is checked. Idle waits are appropriate for this controlled demonstration;
//! a running application with continuous input should use operation-specific
//! responses or progress instead.

use contime::{ConTime, ConTimeConfig, ContimeEvent, ContimeModule, ContimeSnapshot};
use std::{error::Error, io, time::Duration};

// Choose the ID function explicitly. Content ID hashes these event values;
// repeating the same values produces the same ID and is treated as a retry.
#[derive(ContimeEvent)]
#[contime_event(id = |event| contentid::typed_content_id_u128(
    "OrderedValue.v1", &(event.time, event.list_id, u32::from(event.value))
))]
struct OrderedValue {
    time: i64,
    list_id: u128,
    value: char,
}

// The ID closure selects list_id as the key. Matching event fields route to it.
// Its explicit namespace keeps this snapshot identity separate from other types.
// The update rule only appends: ConTime handles ordering and late-event replay.
#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |list_id| contentid::typed_content_id_u128("OrderedList.v1", list_id), events = [OrderedValue], apply = {
    for event in batch.events {
        let OrderedListEvent::OrderedValue(event) = event;
        self.values.push((event.time, event.value));
    }
})]
struct OrderedList {
    time: i64,
    list_id: u128,
    values: Vec<(i64, char)>,
}

// Register both types with one shared timestamp type. Here i64 represents
// logical milliseconds; negative times are ordinary values before the origin.
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [OrderedValue], snapshots = [OrderedList])]
struct Values;

fn main() -> Result<(), Box<dyn Error>> {
    let contime = ConTime::<Values>::start(configuration())?;
    let result = run_example(&contime);
    let shutdown_result = shutdown(contime);

    let values = result?;
    shutdown_result?;
    println!("Verified out-of-order insertion: {values:?}");
    Ok(())
}

fn run_example(contime: &ConTime<Values>) -> Result<Vec<(i64, char)>, Box<dyn Error>> {
    // 1. Start with c at time 30. It is the only value in the list.
    contime.apply([OrderedValue { time: 30, list_id: 7, value: 'c' }.into()]).map_err(api_error)?;
    let values = wait_and_query_list(contime)?;
    assert_eq!(values, [(30, 'c')]);

    // 2. Insert a at time -10. It arrives later, but belongs before c.
    contime.apply([OrderedValue { time: -10, list_id: 7, value: 'a' }.into()]).map_err(api_error)?;
    let values = wait_and_query_list(contime)?;
    assert_eq!(values, [(-10, 'a'), (30, 'c')]);

    // 3. Insert b at time 10. ConTime rebuilds the list with b between a and c.
    contime.apply([OrderedValue { time: 10, list_id: 7, value: 'b' }.into()]).map_err(api_error)?;
    let values = wait_and_query_list(contime)?;
    assert_eq!(values, [(-10, 'a'), (10, 'b'), (30, 'c')]);

    Ok(values)
}

// The walkthrough stops submitting before each wait. This makes idle waiting
// useful here; a continuously running application should use responses or progress.
fn wait_and_query_list(contime: &ConTime<Values>) -> Result<Vec<(i64, char)>, Box<dyn Error>> {
    // Process and query through time 30 after each insertion.
    contime.advance_to(30).map_err(api_error)?;
    contime.wait_until_idle(Duration::from_secs(5))?;
    assert!(contime.errors().try_iter().next().is_none(), "every insertion must be accepted");

    let snapshots = contime.query_at(30, [OrderedList::snapshot_id(7)]).map_err(api_error)?;
    assert_eq!(snapshots.len(), 1, "the registered list must exist");
    let ValuesSnapshotLanes::OrderedList(list) = &*snapshots[0];
    assert_eq!(list.list_id, 7);
    Ok(list.values.clone())
}

// The configuration and lifecycle helpers below keep the walkthrough focused
// on submitting events and checking state.
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
fn shutdown(contime: ConTime<Values>) -> io::Result<()> {
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
