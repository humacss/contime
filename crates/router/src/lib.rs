//! Deterministic dispatch of caller-owned messages to snapshot-owning workers.
//!
//! Start with [`Placement`] to select stable history ownership, then implement
//! [`RoutableInput`], [`RouteInputBatch`], and [`WorkerOutput`] for apply routing
//! with [`route()`]. For a unified apply/query/control queue, implement
//! [`RouteInput`] and use [`route_messages`]. The default [`InputBatch`],
//! [`RoutedInput`], and [`WorkerBatch`] containers are optional adapters.
//!
//! Routing consumes messages. Fan-out clones input and completion handles only
//! when additional destinations need them; this crate chooses no shared-pointer
//! representation. Dispatch success means worker messages were sent, not that
//! workers processed them. A failed send can follow successful sends to earlier
//! workers and does not roll them back. Separate router loops do not establish
//! global order; coordination and completion policies belong to the caller.

#![deny(missing_docs)]

mod advance;
mod listen;
mod placement;
mod query;
mod route;
mod types;

pub use self::advance::route_advance;
pub use self::listen::route_snapshot_listeners;
pub use self::placement::Placement;
pub use self::query::{route_event_query, route_snapshot_query};
pub use self::route::{route, route_messages};
pub use self::types::{
    AdvanceInput, AdvanceWorkerOutput, EventQueryInput, EventQueryWorkerOutput, InputBatch, RoutableInput, RouteInput, RouteInputBatch,
    RouteInputKind, RouteOutput, RoutedInput, RouterError, SnapshotListenInput, SnapshotListenWorkerOutput, SnapshotQueryInput,
    SnapshotQueryWorkerOutput, WorkerBatch, WorkerOutput,
};
pub use self::types::{CoordinationOutput, Flush};
