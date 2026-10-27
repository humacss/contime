//! Continuous-time state with managed background or caller-driven processing.
//!
//! The default `runtime` feature starts managed background processing. With
//! `default-features = false, features = ["manual"]`, the host drives processing
//! through `contime::manual` without depending on the managed Runtime.
#![deny(missing_docs)]
#![cfg_attr(
    feature = "runtime",
    doc = r###"Continuous-time state from delayed, duplicated, and out-of-order events.

Start with [`ConTime`], the owned in-memory pipeline. Implement [`Input`] to
give each event a stable ID and destination snapshots; implement
[`checkpoints::Snapshot`] and [`checkpoints::ApplyEvents`] to describe the
state and its deterministic application rules. Your timestamp implements
[`checkpoints::Timestamp`] and [`AdvanceTime`]. Independent histories are
distributed across workers; a history is reconstructed in canonical event order.

# Getting started

The complete example below defines counter events and snapshots, starts four
workers, and queries two counters at 20 Hz while a producer submits changes.
Some events affect both counters. It runs for five seconds; `fastrand` only
varies its example traffic and is not required by ConTime.
Pass `()` as the application wrapper unless you need the hooks in
[`checkpoints::ApplyWrapper`]. No application framework or transport is required.

# Completion and retention

[`ConTime::apply`] queues admission; a successful return does not mean that
events have been applied. [`ConTime::errors`] reports asynchronous rejections.
[`ConTime::send`] gives a caller its own insertion response channel instead.
Insertion responses establish insertion, not completed replay. Running
applications should observe operation-specific responses or progress rather
than waiting for global idle. [`ConTime::wait_until_idle`] is a helper for
controlled tests and benchmarks only. Queries observe available retained
history and do not wait for global idle or publish application effects.

[`ConTime::advance_to`] raises the processing target without waiting for replay.
It also requests time-based retention through [`ConTimeConfig::history_retention`].
[`ConTime::subscribe_pruned_horizon`] observes completed pruning, while
[`ProgressPolicy`] supplies separate local processing observations.
Neither a query nor a progress observation establishes external delivery or
durable storage. ConTime owns no persistence or network protocol.

Shut down the owned topology with [`ConTime::shutdown`] after stopping
submissions; inspect its [`ShutdownReport`] for router and worker outcomes.

# Example

Run this example with `cargo run -p contime --example counter`."###
)]
#![cfg_attr(feature = "runtime", doc = concat!("```rust\n", include_str!("../examples/cases/counter/main.rs"), "\n```\n"))]

/// Shared application contracts and synchronous storage adapters.
pub use contime_core as core;
pub use contime_core::{checkpoints, lanes, AdvanceTime, Input, RejectionReason};
/// Caller-driven processing; creating an instance starts no background threads.
#[cfg(feature = "manual")]
pub mod manual;
#[cfg(feature = "runtime")]
pub use contime_runtime::{
    ApiError, ApplyResponse, ConTime, ConTimeConfig, IdleError, Placement, ProgressObservation, ProgressPolicy, RejectionMessage,
    SharedEvent, ShutdownReport, SnapshotListenerMessage, StartError, ThreadInput, ThreadOutcome, WorkerCounters, WorkerStatus,
};

/// Worker observations and application accounting, separate from presentation.
#[cfg(feature = "monitor")]
pub use contime_monitor as monitor;
/// Caller-driven load simulation and adaptive Maximum admission.
#[cfg(feature = "simulator")]
pub use contime_simulator as simulator;
