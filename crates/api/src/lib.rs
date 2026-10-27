//! API-side request adapters and rejection collection for ConTime.
//!
//! The optional `manual` feature adds local callback-based worker requests.
//! It starts no processing and owns no storage or worker loop.
//!
//! The API forwards one batch per call to an opaque downstream receiver and
//! collects insertion rejections until request senders close. Replay completion
//! is defined by the downstream adapter. Rejection reasons are generic, and the crate has no knowledge
//! of downstream processing topology or the root `contime` crate.
//!
//! Use [`send`] and the `send_*` helpers to provide response channels yourself.
//! [`apply`], [`query_at`], [`query_events_between`] and [`advance_to`] own
//! their channels and block until every downstream sender is dropped. Output
//! traits such as [`ApplyOutput`] and [`SnapshotQueryOutput`] adapt the boundary
//! to an application's message types without coupling it to processing storage.
//! A successful send acknowledges enqueueing only; downstream adapters define
//! processing completion and the interpretation of timestamps and rejections.
//!
//! # Forwarding a batch
//!
//! ```
//! use contime_api::{send, InputBatch};
//! use crossbeam_channel::unbounded;
//!
//! let (requests, downstream) = unbounded::<InputBatch<u64, &str>>();
//! let (rejections, rejected) = unbounded();
//! send(&requests, [3_u64, 5], rejections).unwrap();
//! let batch = downstream.recv().unwrap();
//! assert_eq!(batch.inputs, vec![3, 5]);
//! // A downstream adapter reports rejections through the batch's sender.
//! drop(batch);
//! assert!(rejected.recv().is_err());
//! ```

#![deny(missing_docs)]

mod advance_to;
mod apply;
mod query_at;
mod query_events_between;
mod send;
mod send_advance_to;
mod send_listen_snapshots;
mod send_query_at;
mod send_query_events_between;
mod types;

pub use advance_to::advance_to;
pub use apply::apply;
pub use query_at::query_at;
pub use query_events_between::query_events_between;
pub use send::send;
pub use send_advance_to::send_advance_to;
pub use send_listen_snapshots::send_listen_snapshots;
pub use send_query_at::send_query_at;
pub use send_query_events_between::send_query_events_between;
pub use types::{
    AdvanceOutput, ApiError, ApplyOutput, ApplyResponse, EventQueryOutput, InputBatch, RejectionMessage, SnapshotListenOutput,
    SnapshotQueryOutput,
};

#[cfg(feature = "manual")]
pub mod manual;
