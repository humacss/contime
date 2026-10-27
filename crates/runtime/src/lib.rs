//! Managed background ConTime execution, admission and thread lifecycle.
//!
//! [`ConTime::start`] starts the complete runtime. The root `contime` crate
//! re-exports this interface. Shared application/storage contracts live in Core.
//! [`ThreadTopology`] is the low-level generic topology used by process adapters
//! and diagnostics; it does not interpret messages or coordinate admission.
#![deny(missing_docs)]
mod checkpoint;
mod coordinator;
mod frontier;
mod input;
mod instance;
#[cfg(test)]
mod listen;
mod message;
#[cfg(test)]
#[path = "../tests/controlled/replay_frontier.rs"]
mod replay_frontier_tests;
mod router;
#[cfg(test)]
mod testing;
mod topology;
mod types;
mod worker;
pub use contime_api::{ApiError, ApplyResponse, RejectionMessage};
pub use contime_core::{checkpoints, lanes, Input, RejectionReason};
pub use contime_progress::{ProgressObservation, ProgressPolicy};
pub use contime_router::Placement;
pub use contime_worker::AdvanceTime;
pub use instance::{ConTime, ConTimeConfig, IdleError};
/// A managed background ConTime instance.
pub type Runtime<I, S, W> = ConTime<I, S, W>;
pub use coordinator::{run as coordinate, Settings as CoordinatorSettings};
pub use topology::{Router, RuntimeStage, ShutdownReport, StartError, ThreadOutcome, ThreadTopology, TopologySendError, Worker};
pub use types::{
    Advance, CompletionHandle, EventQuery, Route, RouterBatch, RouterMessage, RouterProcess, SharedEvent, SnapshotListen, SnapshotListener,
    SnapshotListenerMessage, SnapshotQuery, WorkerBatch, WorkerMessage, WorkerProcess,
};
/// An input that can cross the managed runtime's thread boundaries.
/// Core application and storage contracts require only [`Input`].
pub trait ThreadInput: Input<Time: Send + Sync> + Send + Sync {}
impl<I> ThreadInput for I
where
    I: Input + Send + Sync,
    I::Time: Send + Sync,
{
}
