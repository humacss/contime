#![doc = concat!(include_str!("../assets/contime-logo.svg"), "\n\n")]
//! Continuous-time state with managed background or caller-driven processing.
//!
//! Runtime is enabled by default. Start managed processing with `ConTime::start`.
//! Enable `manual` to drive a worker yourself through `contime::manual`; creating
//! a Manual instance starts no threads. For Manual alone, disable default features
//! and select `features = ["manual", "time"]` for integer time, or just `manual`
//! for an application-owned time type. The optional `monitor` feature also enables
//! worker application counts. The independent `simulator` feature provides event
//! generation; enable both tools for adaptive monitoring-based load control.
#![deny(missing_docs)]
#![cfg_attr(
    feature = "macros",
    doc = r###"
# Declarative modules

With `macros`, derive [`ContimeEvent`], [`ContimeSnapshot`] and [`ContimeModule`].
A module selects the time type and combines its events, snapshots and dependencies.
Start managed processing with `ConTime::<App>::start(config)` or drive the same
module through `manual::ManualConTime::<App>::new(config)` with `manual` enabled.
All nested declarations must agree on the time type. Generated event and snapshot
lanes remain available for typed submission and query results.

The default `time` feature supplies integer time implementations, including the
full signed range. Disabling it leaves `time::Timestamp` and `time::AdvanceTime`
available for custom types. The explicit minimum sets the initial history bound;
`Default` does not define the beginning of time.
"###
)]
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

With `macros` enabled, the complete example defines counter events and snapshots
with derive attributes, starts four workers, and queries two counters at 20 Hz
while a producer submits changes.
Some events affect both counters. It runs for five seconds; `fastrand` only
varies its example traffic and is not required by ConTime.
The module facade supplies the ordinary application wrapper. Use
`start_with_wrapper` for custom [`checkpoints::ApplyWrapper`] hooks.
No application framework or transport is required.

# Completion and retention

[`ConTime::apply`](RawConTime::apply) queues admission; a successful return does not mean that
events have been applied. [`ConTime::errors`](RawConTime::errors) reports asynchronous rejections.
[`ConTime::send`](RawConTime::send) gives a caller its own insertion response channel instead.
Insertion responses establish insertion, not completed replay. Running
applications should observe operation-specific responses or progress rather
than waiting for global idle. [`ConTime::wait_until_idle`](RawConTime::wait_until_idle) is a helper for
controlled tests and benchmarks only. Queries observe available retained
history and do not wait for global idle or publish application effects.

[`ConTime::advance_to`](RawConTime::advance_to) raises the processing target without waiting for replay.
It also requests time-based retention through [`ConTimeConfig::history_retention`].
[`ConTime::subscribe_pruned_horizon`](RawConTime::subscribe_pruned_horizon) observes completed pruning, while
[`ProgressPolicy`] supplies separate local processing observations.
Neither a query nor a progress observation establishes external delivery or
durable storage. ConTime owns no persistence or network protocol.

Shut down the owned topology with [`ConTime::shutdown`] after stopping
submissions; inspect its [`ShutdownReport`] for router and worker outcomes.

# Example

Run this example with `cargo run -p contime --example readme --features macros`."###
)]
#![cfg_attr(all(feature = "runtime", feature = "macros"), doc = concat!("```rust\n", include_str!("../examples/cases/counter/readme.rs"), "\n```\n"))]

/// Shared application contracts and storage adapters for either execution mode.
pub use crate::contime_core as core;
pub mod module;
pub use crate::contime_core::module::Module;
pub use crate::contime_core::{checkpoints, lanes, AdvanceTime, Input, RejectionReason};
#[cfg(feature = "runtime")]
pub use crate::contime_runtime::ConTime as RawConTime;
#[cfg(feature = "runtime")]
pub use crate::contime_runtime::ModuleConTime as ConTime;
#[cfg(feature = "macros")]
pub use contime_macros::{ContimeEvent, ContimeModule, ContimeSnapshot};
#[cfg(feature = "macros")]
#[doc(hidden)]
pub use contime_macros::{__contime_collect_module, __contime_register_module};
/// Caller-driven processing; creating an instance starts no background threads.
#[cfg(feature = "manual")]
pub mod manual;
#[cfg(feature = "runtime")]
pub use crate::contime_runtime::{
    ApiError, ApplyResponse, ConTimeConfig, IdleError, Placement, ProgressObservation, ProgressPolicy, RejectionMessage, SharedEvent,
    ShutdownReport, SnapshotListenerMessage, StartError, ThreadInput, ThreadOutcome, WorkerCounters, WorkerStatus,
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
// Components keep normal manifests and authoritative source under crates/.
// src/_symlinks/ exposes that same source through tracked symbolic links,
// without the component manifests. Cargo packages the links as ordinary files.
// Standalone src/lib.rs and embedded src/mod.rs declare the same implementation
// files; no generated release source or second maintained copy is needed.
// See src/_symlinks/README.md for the layout and checkout requirements.
//
// The re-exports above provide the consumer-facing API. The feature gates below
// select the implementations needed for that API, while doc(hidden) keeps these
// assembly entry points out of the normal generated documentation.
#[cfg(any(feature = "runtime", feature = "manual"))]
#[doc(hidden)]
#[path = "_symlinks/api/src/mod.rs"]
pub mod contime_api;
#[cfg(feature = "runtime")]
#[doc(hidden)]
#[path = "_symlinks/batcher/src/mod.rs"]
pub mod contime_batcher;
#[doc(hidden)]
#[path = "_symlinks/snapshots/src/mod.rs"]
pub mod contime_checkpoints;
#[doc(hidden)]
#[path = "_symlinks/core/src/mod.rs"]
pub mod contime_core;
#[doc(hidden)]
#[path = "_symlinks/events/src/mod.rs"]
pub mod contime_events;
#[doc(hidden)]
#[path = "_symlinks/lanes/src/mod.rs"]
pub mod contime_lanes;
#[cfg(feature = "manual")]
#[doc(hidden)]
#[path = "_symlinks/manual/src/mod.rs"]
pub mod contime_manual;
#[cfg(feature = "monitor")]
#[doc(hidden)]
#[path = "_symlinks/monitor/src/mod.rs"]
pub mod contime_monitor;
#[cfg(feature = "runtime")]
#[doc(hidden)]
#[path = "_symlinks/progress/src/mod.rs"]
pub mod contime_progress;
#[cfg(feature = "runtime")]
#[doc(hidden)]
#[path = "_symlinks/router/src/mod.rs"]
pub mod contime_router;
#[cfg(feature = "runtime")]
#[doc(hidden)]
#[path = "_symlinks/runtime/src/mod.rs"]
pub mod contime_runtime;
#[cfg(feature = "simulator")]
#[doc(hidden)]
#[path = "_symlinks/simulator/src/mod.rs"]
pub mod contime_simulator;
#[doc(hidden)]
#[path = "_symlinks/worker/src/mod.rs"]
pub mod contime_worker;

/// Time domain contracts and optional built-in integer implementations.
pub use crate::contime_time as time;
#[doc(hidden)]
#[path = "_symlinks/time/src/mod.rs"]
pub mod contime_time;
