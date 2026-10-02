//! Bounded measurement; callers own fencing, admission and completion policy.

mod coverage;
mod policy;
mod round;

pub use coverage::CoverageConstraint;
pub use policy::{ProgressObservation, ProgressPolicy};
pub use round::MeasurementRound;
