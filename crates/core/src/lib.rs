//! Apply-and-query composition of isolated ConTime subsystems.

mod advance;
mod apply;
mod checkpoint;
mod coordinator;
mod frontier;
mod history;
mod idle;
mod input;
mod listen;
mod message;
mod query;
mod router;
mod send;
mod shutdown;
mod start;
mod types;
mod worker;

pub use types::{
    Advance, CompletionHandle, ConTime, ConTimeConfig, EventQuery, Input, RejectionReason, Route, RouterBatch, RouterMessage,
    RouterProcess, SharedEvent, SnapshotListen, SnapshotListener, SnapshotListenerMessage, SnapshotQuery, WorkerBatch, WorkerMessage,
    WorkerProcess,
};

pub use contime_api::{ApiError, ApplyResponse, RejectionMessage};
pub mod checkpoints;
pub use contime_lanes as lanes;
pub use contime_router::Placement;
pub use idle::IdleError;
