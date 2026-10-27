#![doc = include_str!("../README.md")]
#![deny(missing_docs)]
#![forbid(unsafe_code)]
mod controller;
mod random;
mod simulation;
mod stream;
pub use controller::{Maximum, MaximumConfig, WorkUnit};
pub use random::Random;
pub use simulation::{ProduceError, Query, QueryError, Simulator, Workload};
pub use stream::{EventStream, StreamConfig};
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
    Monitor(contime_monitor::Error),
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
