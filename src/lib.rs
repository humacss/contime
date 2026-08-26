//! `contime` builds queryable continuous-time state from unreliable event streams.
//!
//! Instead of only tracking the latest state, `contime` keeps enough history to answer
//! "what did snapshot `X` look like at time `T`?" while still supporting bounded memory
//! and multi-worker processing.
//!
//! # Workflow
//!
//! 1. Derive [`ContimeEvent`] and [`ContimeSnapshot`] for your domain types, or implement
//!    [`Snapshot`], [`Event`], and [`ApplyEvents`] manually.
//! 2. Generate lane enums with [`lanes!`], or define compatible lane types manually.
//! 3. Construct a [`Contime`] with a worker count and memory budget.
//! 4. Apply events and markers through one generated input lane.
//! 5. Query state with [`Contime::query_at`].
//!
//! Point queries include all events at or before the complete ordered query time.
//! Markers are global temporal records routed into replay batches for custom
//! [`ApplyWrapper`] interpretation; they never apply to snapshots directly.
//!
//! # Where To Start
//!
//! For a runnable custom-type setup, see `examples/ordered_values.rs` and run
//! `cargo run --example ordered_values`.
//!
//! The quick doctest below uses the exported test fixtures to show the `contime` flow end to end
//! with the current public API.
//!
//! ```rust
//! use contime::{Input, TestEvent, TestSnapshot, TestSnapshotContime};
//!
//! let contime = TestSnapshotContime::new(1, 1_024);
//!
//! contime.apply([TestEvent::Positive(1, 5, 10, 3)].map(Into::into)).unwrap();
//!
//! let snapshot: TestSnapshot = contime.query_at(6, &[1]).unwrap().pop().flatten().unwrap().into();
//! assert_eq!(snapshot.sum, 3);
//!
//! contime.apply([TestEvent::Positive(1, 4, 11, 2)].map(Into::into)).unwrap();
//!
//! let snapshot: TestSnapshot = contime.query_at(6, &[1]).unwrap().pop().flatten().unwrap().into();
//! assert_eq!(snapshot.sum, 5);
//!
//! ```
mod api;
mod batch;
mod history;
mod key;
mod rejection;
mod router;
mod time;
mod traits;
mod worker;

extern crate self as contime;

use key::ContimeKey;
use router::{Router, RouterError};
use worker::{Worker, WorkerInbound};

#[doc(hidden)]
pub use api::CompletionBenchmark;
pub use api::{Contime, ContimeError};
#[doc(hidden)]
pub use batch::SnapshotBatchBenchmark;
#[doc(hidden)]
pub use contime_macros::__lanes_merge;
pub use contime_macros::{lanes, ContimeEvent, ContimeSnapshot};
pub use history::{ApplyInner, ApplyWrapper, SnapshotHistory};
#[doc(hidden)]
pub use history::{HistoryInputs, HistoryInsert};
pub use rejection::{EventRejection, EventRejectionReason};
#[doc(hidden)]
pub use router::RoutePartitionBenchmark;
#[doc(hidden)]
pub use router::RouterApplyBenchmark;
pub use time::ContimeTime;
pub use traits::{
    ApplyBatch, ApplyEvents, Event, Input, InputBatch, InputLanes, InputRoute, Marker, Snapshot, SnapshotEvent, SnapshotLanes,
};
#[doc(hidden)]
pub use worker::{WorkerApplyBatch, WorkerApplyBenchmark};

mod test;
pub use test::*;
