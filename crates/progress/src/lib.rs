//! Bounded measurement; callers own fencing, admission and completion policy.
//!
//! [`MeasurementRound`] reduces one report from every participant to an exclusive
//! work boundary capped at the round's cutoff. [`CoverageConstraint`] holds a
//! conservative admission bound until every affected route has reported its work.
//! [`ProgressPolicy`] lets a host request and consume [`ProgressObservation`]s.
//!
//! These reducers do not inspect queues, deliver fences, resume participants or
//! authorize pruning. New work can lower later observations; callers own any
//! monotonic committed frontier and external delivery accounting.
//!
//! # Measuring two participants
//!
//! ```
//! use contime_progress::MeasurementRound;
//! use std::num::NonZeroUsize;
//!
//! let mut round = MeasurementRound::new(7, 100_u64, NonZeroUsize::new(2).unwrap());
//! assert_eq!(round.report(7, 0, Some(40)), None);
//! assert_eq!(round.report(7, 1, None), Some(40));
//! assert!(round.is_complete());
//! // The completed boundary excludes the timestamp bucket at 40.
//! ```

#![deny(missing_docs)]

mod coverage;
mod policy;
mod round;

pub use coverage::CoverageConstraint;
pub use policy::{ProgressObservation, ProgressPolicy};
pub use round::MeasurementRound;
