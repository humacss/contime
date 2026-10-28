//! Development crate compiling the shared ConTime component source.
pub extern crate contime_worker;
#[path = "../src/lib.rs"]
pub mod contime_manual;
pub use contime_manual::*;
