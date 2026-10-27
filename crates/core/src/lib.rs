//! Shared application contracts and synchronous history/storage adapters.
//!
//! Managed execution belongs to `contime-runtime`. Core starts no threads,
//! uses no channel protocol, and imposes no Send/Sync bounds on inputs.
#![deny(missing_docs)]
mod checkpoint;
pub mod checkpoints;
mod history;
mod input;
mod types;
pub use contime_lanes as lanes;
pub use contime_worker::AdvanceTime;
pub use types::{CheckpointStorage, CheckpointStorageConfig, Input, InsertionRejection, RejectionReason, SharedEvent};
