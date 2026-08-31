//! API-side input forwarding and rejection collection for ConTime.
//!
//! The API forwards one batch per call to an opaque downstream receiver and
//! uses rejection-channel closure to detect synchronous apply
//! completion. Rejection reasons are generic, and the crate has no knowledge
//! of downstream processing topology or the root `contime` crate.

mod apply;
mod send;
mod types;

pub use apply::apply;
pub use send::send;
pub use types::{ApiError, ApplyOutput, ApplyResponse, InputBatch, RejectionMessage};
