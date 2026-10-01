//! Bounded measurement; callers own fencing, admission and completion policy.

mod policy;
mod round;

pub use policy::{ProgressObservation, ProgressPolicy};
pub use round::MeasurementRound;
