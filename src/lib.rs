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
//! 4. Apply events.
//! 5. Query state with [`Contime::query_at`] or inspect retained original events
//!    with [`Contime::inspect_events`].
//!
//! Point queries include all events at or before the complete ordered query time.
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
//! use contime::{Event, TestEvent, TestSnapshot, TestSnapshotContime};
//!
//! let contime = TestSnapshotContime::new(1, 1_024);
//!
//! contime.apply_events([TestEvent::Positive(1, 5, 10, 3)]).unwrap();
//!
//! let snapshot: TestSnapshot = contime.query_at(6, &[1]).unwrap().pop().flatten().unwrap().into();
//! assert_eq!(snapshot.sum, 3);
//!
//! contime.apply_events([TestEvent::Positive(1, 4, 11, 2)]).unwrap();
//!
//! let snapshot: TestSnapshot = contime.query_at(6, &[1]).unwrap().pop().flatten().unwrap().into();
//! assert_eq!(snapshot.sum, 5);
//!
//! let events = contime.inspect_events(4..=5).unwrap();
//! assert_eq!(events.len(), 2);
//! assert_eq!(events[0].event.time(), 4);
//! assert_eq!(events[1].event.time(), 5);
//! ```
mod api;
mod history;
mod journal;
mod key;
mod router;
mod time;
mod traits;
mod worker;

extern crate self as contime;

use key::ContimeKey;
use router::{Router, RouterError};
use worker::{Worker, WorkerInbound};

pub use api::{Contime, ContimeError};
#[doc(hidden)]
pub use contime_macros::__lanes_merge;
pub use contime_macros::{lanes, ContimeEvent, ContimeSnapshot};
pub use history::{ApplyDecision, ApplyInner, ApplyWrapper, SnapshotHistory};
pub use journal::EventJournalEntry;
pub use time::ContimeTime;
pub use traits::{ApplyBatch, ApplyEvents, Event, EventLanes, RoutedSnapshot, SeedSnapshot, Snapshot, SnapshotEvent, SnapshotLanes};

mod test;
pub use test::*;
