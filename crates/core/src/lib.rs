//! Protocol and storage adapters connecting isolated ConTime subsystems.

mod checkpoint;
mod coordinator;
mod frontier;
mod history;
mod input;
#[cfg(test)]
mod listen;
mod message;
mod router;
mod types;
mod worker;

#[cfg(test)]
#[path = "../tests/controlled/replay_frontier.rs"]
mod replay_frontier_tests;

pub use types::{
    Advance, CompletionHandle, EventQuery, Input, RejectionReason, Route, RouterBatch, RouterMessage, RouterProcess, SharedEvent,
    SnapshotListen, SnapshotListener, SnapshotListenerMessage, SnapshotQuery, WorkerBatch, WorkerMessage, WorkerProcess,
};

pub use contime_api::{ApiError, ApplyResponse, RejectionMessage};
pub mod checkpoints;
pub use contime_lanes as lanes;
pub use contime_progress::{ProgressObservation, ProgressPolicy};
pub use contime_router::Placement;

pub use coordinator::{run as coordinate, Settings as CoordinatorSettings};
