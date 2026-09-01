//! Deterministic input-batch routing independent of ConTime orchestration.

mod advance;
mod hash;
mod query;
mod route;
mod types;

pub use advance::route_advance;
pub use query::{route_event_query, route_snapshot_query};
pub use route::{route, route_messages};
pub use types::{
    AdvanceInput, AdvanceWorkerOutput, EventQueryInput, EventQueryWorkerOutput, InputBatch, RoutableInput, RouteInput, RouteInputBatch,
    RouteInputKind, RouteOutput, RoutedInput, RouterError, SnapshotQueryInput, SnapshotQueryWorkerOutput, WorkerBatch, WorkerOutput,
};
