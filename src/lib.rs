//! Public orchestration of ConTime's apply-and-query pipeline.

mod advance;
mod api;
mod apply;
mod idle;
mod listen;
mod query;
mod send;
mod shutdown;
mod start;

#[cfg(test)]
mod testing;

pub use api::{ConTime, ConTimeConfig};
pub use contime_api::{ApiError, ApplyResponse, RejectionMessage};
pub use contime_core::{
    checkpoints, lanes, Input, Placement, ProgressObservation, ProgressPolicy, RejectionReason, SharedEvent, SnapshotListenerMessage,
};
pub use contime_runtime::{ShutdownReport, StartError, ThreadOutcome};
pub use idle::IdleError;

use contime_core::{RouterBatch, RouterMessage, RouterProcess, WorkerProcess};
