//! API-side input forwarding and rejection collection for ConTime.
//!
//! The API forwards one batch per call to an opaque downstream receiver and
//! uses rejection-channel closure to detect synchronous apply
//! completion. Rejection reasons are generic, and the crate has no knowledge
//! of downstream processing topology or the root `contime` crate.

mod apply;
mod query_at;
mod query_events_between;
mod send;
mod send_query_at;
mod send_query_events_between;
mod types;

pub use apply::apply;
pub use query_at::query_at;
pub use query_events_between::query_events_between;
pub use send::send;
pub use send_query_at::send_query_at;
pub use send_query_events_between::send_query_events_between;
pub use types::{ApiError, ApplyOutput, ApplyResponse, EventQueryOutput, InputBatch, RejectionMessage, SnapshotQueryOutput};
