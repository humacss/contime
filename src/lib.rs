//! Continuous-time state with managed background or caller-driven processing.
//!
//! Runtime is enabled by default. Start managed processing with `ConTime::start`.
//! Enable `manual` to drive a worker yourself through `contime::manual`; creating
//! a Manual instance starts no threads. For Manual alone, disable default features
//! and select `features = ["manual"]`. The optional `monitor` feature also enables
//! worker application counts. The independent `simulator` feature provides event
//! generation; enable both tools for adaptive monitoring-based load control.
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

Run this example with `cargo run -p contime --example readme`."###
)]
#![cfg_attr(feature = "runtime", doc = concat!("```rust\n", include_str!("../examples/cases/counter/readme.rs"), "\n```\n"))]

/// Shared application contracts and storage adapters for either execution mode.
pub use crate::contime_core as core;
pub use crate::contime_core::{checkpoints, lanes, AdvanceTime, Input, RejectionReason};
/// Caller-driven processing; creating an instance starts no background threads.
#[cfg(feature = "manual")]
pub mod manual;
#[cfg(feature = "runtime")]
pub use crate::contime_runtime::{
    ApiError, ApplyResponse, ConTime, ConTimeConfig, IdleError, Placement, ProgressObservation, ProgressPolicy, RejectionMessage,
    SharedEvent, ShutdownReport, SnapshotListenerMessage, StartError, ThreadInput, ThreadOutcome, WorkerCounters, WorkerStatus,
};

/// Worker observations and application accounting, separate from presentation.
#[cfg(feature = "monitor")]
pub use crate::contime_monitor as monitor;
/// Caller-driven load simulation and adaptive Maximum admission.
#[cfg(feature = "simulator")]
pub use crate::contime_simulator as simulator;

// Why these modules load source from the development crates:
//
// We keep components as separate Cargo crates for isolated tests, benchmarks,
// and clear dependency boundaries, but distribute ConTime as one crates.io
// package. Cargo does not bundle workspace dependencies into a published
// package: internal path dependencies would need separately published registry
// packages. Cargo also excludes nested packages from the parent archive.
//
// Each component therefore keeps its development manifest and thin crate wrapper
// under crates/<component>/dev/, outside the implementation source tree. Those
// development crates are not published. This package includes the shared source
// files and compiles them directly as modules through the paths below. Both
// arrangements use the same implementation; no release-time source generation
// or second copy of the algorithms is needed.
//
// The re-exports above provide the consumer-facing API. The feature gates below
// select the implementations needed for that API, while doc(hidden) keeps these
// assembly entry points out of the normal generated documentation.
#[cfg(any(feature = "runtime", feature = "manual"))]
#[doc(hidden)]
#[path = "../crates/api/src/lib.rs"]
pub mod contime_api;
#[cfg(feature = "runtime")]
#[doc(hidden)]
#[path = "../crates/batcher/src/lib.rs"]
pub mod contime_batcher;
#[doc(hidden)]
#[path = "../crates/snapshots/src/lib.rs"]
pub mod contime_checkpoints;
#[doc(hidden)]
#[path = "../crates/core/src/lib.rs"]
pub mod contime_core;
#[doc(hidden)]
#[path = "../crates/events/src/lib.rs"]
pub mod contime_events;
#[doc(hidden)]
#[path = "../crates/lanes/src/lib.rs"]
pub mod contime_lanes;
#[cfg(feature = "manual")]
#[doc(hidden)]
#[path = "../crates/manual/src/lib.rs"]
pub mod contime_manual;
#[cfg(feature = "monitor")]
#[doc(hidden)]
#[path = "../crates/monitor/src/lib.rs"]
pub mod contime_monitor;
#[cfg(feature = "runtime")]
#[doc(hidden)]
#[path = "../crates/progress/src/lib.rs"]
pub mod contime_progress;
#[cfg(feature = "runtime")]
#[doc(hidden)]
#[path = "../crates/router/src/lib.rs"]
pub mod contime_router;
#[cfg(feature = "runtime")]
#[doc(hidden)]
#[path = "../crates/runtime/src/lib.rs"]
pub mod contime_runtime;
#[cfg(feature = "simulator")]
#[doc(hidden)]
#[path = "../crates/simulator/src/lib.rs"]
pub mod contime_simulator;
#[doc(hidden)]
#[path = "../crates/worker/src/lib.rs"]
pub mod contime_worker;
