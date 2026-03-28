//! `contime` builds queryable continuous-time state from unreliable event streams.
//!
//! Instead of only tracking the latest state, `contime` keeps enough history to answer
//! "what did snapshot `X` look like at time `T`?" while still supporting bounded memory
//! and multi-worker processing.
//!
//! # Workflow
//!
//! 1. Implement [`Snapshot`], [`Event`], and [`ApplyEvent`] for your domain types.
//! 2. Generate lane enums with [`contime!`] or define compatible lane types manually.
//! 3. Construct a [`Contime`] with a worker count and memory budget.
//! 4. Apply events or authoritative snapshots.
//! 5. Query state with [`Contime::at`] or [`Contime::query_at`] and listen for
//!    [`Reconciliation`] notifications when late data changes previously queried history.
//!
//! Queries currently replay events with `event.time() < query_time`, so query the first
//! logical instant after the events you want included.
//!
//! # Where To Start
//!
//! The README includes a full custom-type setup. The example below uses the exported test
//! fixtures to show the runtime flow end to end with the current public API.
//!
//! ```rust
//! use contime::{TestEvent, TestSnapshot, TestSnapshotContime};
//!
//! let contime = TestSnapshotContime::new(1, 1_024);
//!
//! contime.apply_event(TestEvent::Positive(1, 5, 10, 3)).unwrap();
//!
//! let (snapshot, reconciliation_rx) = contime.at::<TestSnapshot>(6, 1).unwrap();
//! assert_eq!(snapshot.sum, 3);
//!
//! contime.apply_event(TestEvent::Positive(1, 4, 11, 2)).unwrap();
//!
//! let reconciliation = reconciliation_rx.recv().unwrap();
//! assert_eq!(reconciliation.snapshot_id, 1);
//! assert_eq!(reconciliation.from_time, 4);
//! assert_eq!(reconciliation.to_time, 5);
//! ```
mod api;
mod apply;
mod handle;
mod history;
mod key;
mod macros;
mod router;
mod traits;
mod worker;

use apply::apply_event_in_place;
use key::ContimeKey;
use router::{Router, RouterError};
use worker::{Worker, WorkerInbound};

pub use api::{Contime, ContimeError};
pub use handle::{AdvanceHandle, ApplyHandle, HandleError, QueryHandle, QueryResult};
pub use history::{Reconciliation, SnapshotHistory};
pub use traits::{ApplyEvent, Event, EventLanes, Snapshot, SnapshotLanes};

mod test;
pub use test::*;
