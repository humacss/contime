//! Development crate compiling the shared ConTime component source.
#[cfg(feature = "runtime")]
pub extern crate contime_runtime;
#[cfg(feature = "monitor")]
pub extern crate contime_worker;
#[path = "../src/lib.rs"]
pub mod contime_monitor;
pub use contime_monitor::*;
