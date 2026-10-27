#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod monitor;
mod observation;

pub use monitor::Monitor;
pub use observation::{Error, Observation, Query, Rates, Snapshot, Totals};

#[cfg(feature = "workers")]
mod worker_monitor;
#[cfg(feature = "workers")]
mod worker_observation;
#[cfg(feature = "workers")]
pub use contime_worker::{WorkerCounters, WorkerStatus};
#[cfg(feature = "workers")]
pub use worker_monitor::WorkerMonitor;
#[cfg(feature = "workers")]
pub use worker_observation::{WorkerMonitorError, WorkerReading, WorkerStatusSource};

#[cfg(feature = "workers")]
mod worker_activity;
#[cfg(feature = "workers")]
pub use worker_activity::{ApplicationActivity, WorkerActivity};
