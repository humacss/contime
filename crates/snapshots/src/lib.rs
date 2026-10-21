//! Reconstruct historical snapshot state from canonical event histories.
//!
//! [`SnapshotStore`] owns a consumer-provided [`EventStore`] and materialized
//! [`Checkpoint`] values. Implement [`Snapshot`] for clonable state, [`Event`]
//! for timestamp access, and [`Apply`] for deterministic timestamp-batch
//! application. [`InsertEventStore`] additionally enables admission of events
//! after construction.
//!
//! # Choosing an operation
//!
//! - [`SnapshotStore::query`] reconstructs an owned snapshot through an inclusive
//!   target, leaving retained checkpoints and processing progress unchanged.
//! - [`SnapshotStore::process_until`] retains reconstructed progress through an
//!   inclusive target and exposes [`BatchLookahead`] to application callbacks.
//! - [`SnapshotStore::insert`] admits events and invalidates affected checkpoints
//!   without invoking application logic. Late events are replayed on demand.
//! - [`SnapshotStore::forward`] retains state immediately before a new horizon.
//!   It requires [`Timestamp`] and may invoke a snapshot-compaction hook.
//! - [`SnapshotStore::prune`] physically removes obsolete history after forwarding.
//!
//! # Boundaries and effects
//!
//! Checkpoint spacing counts raw events and always finishes a whole timestamp;
//! zero spacing means an unbounded interval. Initial state precedes all supplied
//! events, including its own timestamp. Its timestamp is the initial retention
//! horizon. Requests before the horizon fail with [`NoCheckpoint`].
//!
//! Queries still call consumer application logic: supply a context that suppresses
//! external effects. Application and forwarding hooks can run during reconstruction
//! and are not transactional. This crate supplies no workers, event identity
//! scheme, persistent storage, or automatic retention policy.

#![deny(missing_docs)]

mod api;
mod apply;
mod commit;
mod forward;
mod playback;
mod query;
mod replay;
mod store;
mod types;

pub use api::SnapshotStore;
pub use forward::ForwardError;
pub use types::{Apply, BatchLookahead, Checkpoint, Event, EventStore, Insert, InsertEventStore, NoCheckpoint, Snapshot, Timestamp};

pub(crate) use commit::Commit;
pub(crate) use playback::Playback;
pub(crate) use store::Store;
