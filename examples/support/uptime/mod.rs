//! Shared caller-driven service uptime workload; hosts own clocks and presentation.
#![allow(dead_code, unused_imports)]
pub use crate::support::uptime_config::Config;
mod driver;
mod json;
mod reference;
pub mod report;
pub use driver::Driver;
pub use json::config;
pub use report::Report;
