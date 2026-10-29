//! Development crate compiling the shared ConTime component source.
#[cfg(feature = "monitor")]
pub extern crate contime_monitor;
#[path = "../src/lib.rs"]
pub mod contime_simulator;
pub use contime_simulator::*;
