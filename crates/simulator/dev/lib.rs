//! Development crate compiling the shared ConTime component source.
pub extern crate contime_monitor;
#[path = "../src/lib.rs"]
pub mod contime_simulator;
pub use contime_simulator::*;
