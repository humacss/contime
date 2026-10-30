//! Caller-driven event generation, validation, and adaptive traffic admission.
#![deny(missing_docs)]
#![forbid(unsafe_code)]
#[cfg(feature = "monitor")]
mod controller;
mod error;
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
