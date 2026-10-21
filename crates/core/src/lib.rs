//! Protocol and storage adapters connecting isolated ConTime subsystems.
//!
//! The root `contime` crate assembles the running pipeline. This crate supplies
//! its concrete protocol and application seams without starting threads itself.
//!
//! Implement [`Input`] for event identity and snapshot routing, then implement
//! [`checkpoints::ApplyEvents`] for deterministic state application. Optional
//! [`checkpoints::ApplyWrapper`] hooks filter batches, publish live replay effects
//! or compact retained state. [`SharedEvent`] shares one immutable input across
//! its routes; [`RouterMessage`] and [`WorkerMessage`] carry pipeline requests.
//!
//! [`RouterProcess`] and [`WorkerProcess`] adapt those messages to the runtime.
//! [`coordinate`] owns admission, fenced progress measurement and safe pruning on
//! its caller's thread. [`ProgressPolicy`] receives local observations; it does
//! not establish delivery guarantees outside the pipeline. [`lanes`] exposes
//! optional typed consumer application contracts.

#![deny(missing_docs)]

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
