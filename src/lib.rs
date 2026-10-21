//! Continuous-time state from delayed, duplicated, and out-of-order events.
//!
//! Start with [`ConTime`], the owned in-memory pipeline. Implement [`Input`] to
//! give each event a stable ID and destination snapshots; implement
//! [`checkpoints::Snapshot`] and [`checkpoints::ApplyEvents`] to describe the
//! state and its deterministic application rules. Your timestamp implements
//! [`checkpoints::Timestamp`] and [`AdvanceTime`]. Independent histories are
//! distributed across workers; a history is reconstructed in canonical event order.
//!
//! # Getting started
//!
//! The complete example below defines one history containing an ordered list
//! of values. A late event changes the appropriate historical queries, and
//! resubmitting an identical retained event does not apply it twice.
//! Pass `()` as the application wrapper unless you need the hooks in
//! [`checkpoints::ApplyWrapper`]. No application framework or transport is required.
//!
//! # Completion and retention
//!
//! [`ConTime::apply`] queues admission; a successful return does not mean that
//! events have been applied. [`ConTime::errors`] reports asynchronous rejections.
//! [`ConTime::send`] gives a caller its own insertion response channel instead.
//! Stop external producers and use [`ConTime::wait_until_idle`] when an assertion
//! or shutdown sequence needs completed processing. Queries observe available
//! retained history and do not wait for global idle or publish application effects.
//!
//! [`ConTime::advance_to`] raises the processing target without waiting for replay.
//! It also requests time-based retention through [`ConTimeConfig::history_retention`].
//! [`ConTime::subscribe_pruned_horizon`] observes completed pruning, while
//! [`ProgressPolicy`] supplies separate local processing observations.
//! Neither a query nor a progress observation establishes external delivery or
//! durable storage. ConTime owns no persistence or network protocol.
//!
//! Shut down the owned topology with [`ConTime::shutdown`] after stopping
//! submissions; inspect its [`ShutdownReport`] for router and worker outcomes.
//!
//! # Example
//!
//! Run this example with `cargo run -p contime --example ordered_values`.

#![doc = concat!("```rust\n", include_str!("../examples/ordered_values.rs"), "\n```\n")]
#![deny(missing_docs)]

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
pub use contime_worker::AdvanceTime;
pub use idle::IdleError;

use contime_core::{RouterBatch, RouterMessage, RouterProcess, WorkerProcess};
