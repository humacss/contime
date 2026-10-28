#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod monitor;
mod observation;

pub use self::monitor::Monitor;
pub use self::observation::{Error, Observation, Query, Rates, Snapshot, Totals};

#[cfg(feature = "monitor")]
mod worker_monitor;
#[cfg(feature = "monitor")]
mod worker_observation;
#[cfg(feature = "monitor")]
pub use self::worker_monitor::WorkerMonitor;
#[cfg(feature = "monitor")]
pub use self::worker_observation::{WorkerMonitorError, WorkerReading, WorkerStatusSource};
#[cfg(feature = "monitor")]
pub use crate::contime_worker::{WorkerCounters, WorkerStatus};

#[cfg(feature = "monitor")]
mod worker_activity;
#[cfg(feature = "monitor")]
pub use self::worker_activity::{ApplicationActivity, WorkerActivity};
