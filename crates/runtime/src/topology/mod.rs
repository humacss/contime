//! Thread and channel lifecycle for opaque router and worker processes.
//!
//! Implement [`Router`] and [`Worker`], supply nonempty collections to
//! [`ThreadTopology::start`], and submit messages through [`ThreadTopology::send`] or
//! [`ThreadTopology::input`]. Routers compete for a shared unbounded input queue; each
//! worker has a private unbounded queue receiving from every router.
//! This crate does not interpret messages or impose ordering across routers.
//!
//! [`ThreadTopology::shutdown`] closes the owned sender and joins routers before
//! workers, collecting a [`ShutdownReport`] even when a process fails or panics.
//! Drop external sender clones before shutdown so receivers can disconnect.
//! Process implementations must release their channels and return for joining
//! to finish; this crate neither cancels nor restarts them. Dropping a runtime
//! without explicit shutdown drops its join handles and does not join threads.

#![deny(missing_docs)]

mod runtime;
mod shutdown;
mod start;
mod types;

pub use runtime::TopologySendError;
pub use types::{Router, RuntimeStage, ShutdownReport, StartError, ThreadOutcome, ThreadTopology, Worker};
