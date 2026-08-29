//! Apply-only runtime topology independent of ConTime domain crates.

mod runtime;
mod shutdown;
mod start;
mod types;

pub use types::{
    Router, Runtime, RuntimeConfig, RuntimeStage, ShutdownReport, StartError, ThreadOutcome, Worker,
};
