//! Static projection, filtering, and application of complete event batches.
//!
//! A lane family implements [`Lanes`] with event and batch types that can borrow
//! canonical input. [`FilterLanes`] projects raw events for a snapshot, and
//! [`EventFilter`] transforms that input into an [`ApplyLanes`] family consumed
//! by [`ApplyEvents`]. [`PassThrough`] forwards the same batch representation
//! without allocating when both families coincide.
//!
//! # Pipeline
//!
//! Construct a [`RawBatch`] from a complete, canonically ordered timestamp bucket
//! and use [`project`] to obtain a [`FilterBatch`]. [`apply()`] runs the filter and
//! delivers its single [`ApplyBatch`] synchronously to the snapshot. A filter
//! must emit exactly one batch, even when it is empty. [`filter()`] also supports
//! custom [`FilterOutput`] receivers.
//!
//! Snapshot ID, timestamp, and cumulative raw history count travel unchanged
//! through the pipeline. The effective event count may change through projection
//! or filtering. Callers supply ordering, timestamp grouping, and progress; this
//! crate does not store events or validate batch metadata. [`Route`] separately
//! connects event destinations to default snapshot construction.

#![deny(missing_docs)]

#[doc(hidden)]
pub extern crate self as contime_lanes;

mod apply;
mod filter;
mod route;
mod types;

pub use self::apply::apply;
pub use self::filter::{filter, project};
pub use self::types::{
    ApplyBatch, ApplyEvents, ApplyLanes, EventFilter, FilterBatch, FilterLanes, FilterOutput, Lanes, PassThrough, RawBatch, Route,
};
