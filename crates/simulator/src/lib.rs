#![doc = include_str!("../README.md")]
#![deny(missing_docs)]
#![forbid(unsafe_code)]
#[cfg(feature = "monitor")]
mod controller;
mod random;
#[cfg(feature = "monitor")]
mod simulation;
mod stream;
#[cfg(feature = "monitor")]
pub use self::controller::{Maximum, MaximumConfig, WorkUnit};
pub use self::random::Random;
#[cfg(feature = "monitor")]
pub use self::simulation::{ProduceError, Query, QueryError, Simulator, Workload};
pub use self::stream::{EventStream, StreamConfig};
/// Invalid simulator configuration, clock, or admission operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Rates, bounds, or durations are zero or inconsistent.
    InvalidConfig,
    /// The next event exceeds the configured whole-event bound.
    EventTooLarge,
    /// A previous submission failed after generation changed the reference.
    Poisoned,
    /// Cumulative simulation totals cannot be represented.
    CounterOverflow,
    /// Invalid query token or monitor clock.
    #[cfg(feature = "monitor")]
    Monitor(crate::contime_monitor::Error),
    /// The supplied elapsed clock moved backwards.
    ClockReversed,
    /// More work was consumed than the pacing credit permits.
    InsufficientCredit,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
