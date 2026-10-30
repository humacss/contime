//! Compose dependent modules into one ConTime instance.
//!
//! Run with `cargo run --example modules --features macros`.
//! Start with `ordered_list.rs` for the smaller step-by-step walkthrough.
//!
//! Values registers the ordered list. Statistics depends on Values and adds a
//! count snapshot; App includes both modules. Their shared Values dependency is
//! registered once. Each input updates the list and its statistics, whose snapshot
//! IDs remain distinct even though they share the same logical list_id.
//!
//! The first query observes a, c. A late b changes the list to a, b, c and the
//! count from two to three. Request-scoped insertion responses precede queries.
//! Application time uses logical milliseconds, including negative timestamps.

use contime::{ConTime, ConTimeConfig, ContimeEvent, ContimeModule, ContimeSnapshot};
use std::{error::Error, io, time::Duration};

#[derive(ContimeEvent)]
#[contime_event(id = |event| contentid::typed_content_id_u128(
    "OrderedValue.v1", &(event.time, event.list_id, u32::from(event.value))
))]
struct OrderedValue {
    time: i64,
    list_id: u128,
    value: char,
}

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

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |list_id| contentid::typed_content_id_u128("ValueStatistics.v1", list_id), events = [OrderedValue], apply = {
    for event in batch.events {
        let ValueStatisticsEvent::OrderedValue(_) = event;
        self.count += 1;
    }
})]
struct ValueStatistics {
    time: i64,
    list_id: u128,
    count: usize,
}

#[derive(ContimeModule)]
#[contime_module(time = i64, events = [OrderedValue], snapshots = [OrderedList])]
#[allow(dead_code)]
struct Values;

#[derive(ContimeModule)]
#[contime_module(time = i64, snapshots = [ValueStatistics], modules = [Values])]
#[allow(dead_code)]
struct Statistics;

// Values is reachable twice. Each event and snapshot lane must occur once.
#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [Values, Statistics])]
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
    let history = ConTime::<App>::start(config)?;
    let result = (|| -> Result<(), Box<dyn Error>> {
        // First establish a, c; then insert b between them in the past.
        for inputs in [
            vec![
                AppEventLanes::from(OrderedValue { time: -10, list_id: 7, value: 'a' }),
                OrderedValue { time: 30, list_id: 7, value: 'c' }.into(),
            ],
            vec![OrderedValue { time: 10, list_id: 7, value: 'b' }.into()],
        ] {
            // This request's channel closes after insertion, so its query can
            // observe the admitted inputs without waiting for global idle.
            let (response, rejected) = crossbeam_channel::unbounded();
            history.send(inputs, response).map_err(api_error)?;
            if let Some(rejection) = rejected.into_iter().next() {
                return Err(io::Error::other(format!("input rejected: {rejection:?}")).into());
            }
            history.advance_to(30).map_err(api_error)?;
            let snapshots = history.query_at(30, [OrderedList::snapshot_id(7), ValueStatistics::snapshot_id(7)]).map_err(api_error)?;
            for snapshot in snapshots {
                match *snapshot {
                    AppSnapshotLanes::OrderedList(state) => println!("list {} = {:?}", state.list_id, state.values),
                    AppSnapshotLanes::ValueStatistics(state) => println!("list {} received {} records", state.list_id, state.count),
                }
            }
        }
        Ok(())
    })();
    // Join the owned background threads on either success or application error.
    let report = history.shutdown();
    result?;
    if report.routers.iter().any(|outcome| *outcome != contime::ThreadOutcome::Completed)
        || report.workers.iter().any(|outcome| *outcome != contime::ThreadOutcome::Completed)
    {
        return Err(io::Error::other("a ConTime thread did not complete normally").into());
    }
    Ok(())
}

fn api_error(error: contime::ApiError) -> io::Error {
    io::Error::other(format!("ConTime request failed: {error:?}"))
}
