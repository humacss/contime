//! Browser support adds worker monitoring without terminal or thread scheduling.
#[path = "../../../support/portable/mod.rs"]
mod portable;
pub use portable::*;
#[path = "../../../support/monitored/portable.rs"]
pub mod monitored;
#[path = "../../../support/monitored/metrics.rs"]
pub mod monitored_metrics;
