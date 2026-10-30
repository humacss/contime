//! Drive one existing ConTime worker synchronously from the host loop.
//!
//! Enqueue caller-owned worker messages, then call [`ManualConTime::process`] once per host-loop turn.
//! [`ManualConTime::process_with_clock`] is an optional budgeted helper.
//! Queries, replay calls, and per-history pruning calls finish normally. Limits
//! are checked between calls; no snapshot operation is suspended. The worker owns
//! all histories and scheduling. Use its public protocol and storage contracts.
//!
//! Pruning is a barrier: prior replay finishes before authorization, and later
//! messages wait for the prune pass. A horizon beyond the accepted processing
//! target returns [`ProcessError`]. User callbacks and response handlers must not
//! block waiting for this same driver. Panics propagate; discard a driver after
//! catching a storage/application panic, just as for the native worker.
#![deny(missing_docs)]
#[doc(hidden)]
pub extern crate contime_time;
#[doc(hidden)]
pub extern crate contime_worker;
#[doc(hidden)]
pub extern crate self as contime_manual;
mod driver;
pub use self::driver::{FrameBudget, FrameReport, ManualConTime, ProcessError, StopReason};
