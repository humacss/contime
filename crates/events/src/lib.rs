//! Canonically ordered, deduplicated event histories with bounded retention.
//!
//! Implement [`Event`] for the value or shared handle you want to retain, then
//! insert it into an [`EventHistory`]. Iteration merges ordered and late arrivals
//! by [`EventKey`]: timestamp first, then event ID. A retained ID is unique across
//! all timestamps; inserting it again leaves the original event unchanged.
//!
//! # Reading and replay
//!
//! [`EventHistory::iter`] borrows the entire retained history, while
//! [`EventHistory::clone_between`] returns owned values over `[from, to)`.
//! [`EventHistory::iter_from_dirty`] includes the whole earliest changed timestamp
//! bucket. A replay consumer calls [`EventHistory::mark_replayed`] to acknowledge
//! the changes; this crate does not apply events or construct snapshot state.
//!
//! # Retention
//!
//! [`EventHistory::prune_before`] monotonically advances the horizon and removes
//! events strictly before it. Events exactly at the horizon remain admissible.
//! Removed IDs are forgotten, so an ID may later be reused at a retained time.
//! Queries only inspect retained events; requesting an older range cannot recover
//! discarded history. There are no threads or automatic memory limits.

#![deny(missing_docs)]

#[doc(hidden)]
pub extern crate contime_time;
#[doc(hidden)]
pub extern crate self as contime_events;
mod advance;
mod history;
mod insert;
mod iteration;
mod query;
mod types;

pub use self::types::{Event, EventHistory, EventHistoryIter, EventHistoryRangeIter, EventKey, Insert, PruneResult};
