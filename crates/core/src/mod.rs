//! Shared application contracts and synchronous history/storage adapters.
//!
//! Execution belongs to `contime-manual` or `contime-runtime`. Core starts no
//! threads, uses no channel protocol, and imposes no Send/Sync bounds on inputs.
#![deny(missing_docs)]
mod checkpoint;
pub mod checkpoints;
mod history;
mod input;
pub mod module;
mod types;
pub use self::types::{CheckpointStorage, CheckpointStorageConfig, Input, InsertionRejection, RejectionReason, SharedEvent};
pub use crate::contime_lanes as lanes;
pub use crate::contime_worker::AdvanceTime;
