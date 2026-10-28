//! Development crate compiling the shared ConTime component source.
#[cfg(feature = "manual")]
pub extern crate contime_worker;
#[path = "../src/lib.rs"]
pub mod contime_api;
pub use contime_api::*;
