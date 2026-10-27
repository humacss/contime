//! WASI composition adds the shared monitored workloads without terminal dependencies.
#[path = "../../../support/portable/mod.rs"]
mod portable;
pub use portable::*;
#[path = "../../../support/monitored/portable.rs"]
pub mod monitored;
#[path = "../../../support/monitored/metrics.rs"]
pub mod monitored_metrics;
#[path = "../../../support/monitored/wire.rs"]
pub mod monitored_wire;
