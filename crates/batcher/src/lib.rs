//! Bounded transport batching independent of submission size and channel type.
//!
//! Implement [`Batch`] to retain application-owned segment context, and [`Request`]
//! to distinguish batchable input from control messages. [`drain`] consumes only
//! immediately available requests and flushes before returning a control boundary.
//! The caller owns admission, dispatch, scheduling, and the processing clock.
//!
//! No threads, channels, sleeps, or ConTime event/storage contracts are required.

#![deny(missing_docs)]

#[doc(hidden)]
pub extern crate self as contime_batcher;

mod batching;
pub use self::batching::{drain, Batch, Batcher, Request, GATHER_WORK, MAX_EVENTS, MAX_REQUESTS};
