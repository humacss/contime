//! Ordered time contracts shared by ConTime storage and execution.
#![deny(missing_docs)]

#[doc(hidden)]
pub extern crate self as contime_time;

// Unit fixtures use integers even when testing a minimal feature selection.
// This does not enable built-ins in the library compiled for consumers.
#[cfg(any(feature = "time", test))]
mod integer;
mod types;

pub use self::types::{AdvanceTime, Timestamp};
