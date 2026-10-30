#![doc = include_str!("../README.md")]
#![deny(missing_docs)]
#![forbid(unsafe_code)]
mod error;
#[cfg(feature = "monitor")]
#[doc(hidden)]
pub extern crate contime_monitor;
#[doc(hidden)]
pub extern crate self as contime_simulator;
#[cfg(feature = "monitor")]
mod controller;
mod random;
#[cfg(feature = "monitor")]
mod simulation;
mod stream;
#[cfg(feature = "monitor")]
pub use self::controller::{Maximum, MaximumConfig, WorkUnit};
pub use self::error::Error;
pub use self::random::Random;
#[cfg(feature = "monitor")]
pub use self::simulation::{ProduceError, Query, QueryError, Simulator, Workload};
pub use self::stream::{EventStream, StreamConfig};
