//! Thread and channel lifecycle for opaque router and worker processes.

mod runtime;
mod shutdown;
mod start;
mod types;

pub use runtime::RuntimeSendError;
pub use types::{Router, Runtime, RuntimeStage, ShutdownReport, StartError, ThreadOutcome, Worker};
