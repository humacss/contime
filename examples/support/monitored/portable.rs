//! Terminal-free composition of the shared native/WASI simulator and monitor driver.
#![allow(dead_code, unused_imports)]
use super::monitored_metrics as metrics;
use crate::definition::Time;
type Failure = Box<dyn std::error::Error>;
#[path = "backend.rs"]
pub mod backend;
#[path = "control.rs"]
pub mod control;
#[path = "driver.rs"]
pub mod driver;
#[path = "options.rs"]
mod options;
#[path = "reference.rs"]
pub mod reference;
#[path = "sampling.rs"]
mod sampling;
pub use options::Options;
pub use sampling::{assign_snapshot_ids, assign_with_primary};
