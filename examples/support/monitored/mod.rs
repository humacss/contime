//! Shared native simulator/monitor host; application models and callbacks stay in examples.
#![allow(dead_code)] // Entry points and Manual hosts use different parts.
use super::{format, ui::input};
use crate::definition::Time;
pub(crate) mod backend;
mod control;
mod driver;
type Failure = Box<dyn std::error::Error>;
use super::monitored_metrics as metrics;
mod host;
mod options;
pub(crate) mod reference;
mod sampling;
use super::monitored_ui as ui;
#[allow(unused_imports)]
pub use driver::Delivery;
#[allow(unused_imports)] // Entry points choose parsed or supplied settings.
pub use host::run_configured;
#[cfg(feature = "manual")]
#[allow(unused_imports)]
pub use host::run_manual_with_options;
#[cfg(feature = "runtime")]
#[allow(unused_imports)]
pub use host::run_with_options;
pub use options::Options;
#[allow(unused_imports)] // Hash generation also needs a shared group primary.
pub use sampling::{assign_snapshot_ids, assign_with_primary};

#[cfg(all(test, feature = "manual"))]
mod backend_tests;
