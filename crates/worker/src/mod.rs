//! Worker-owned histories with incremental, timestamp-ordered replay.
//!
//! Implement [`SnapshotStore`] for storage and application behavior, and
//! [`WorkInput`] for the message protocol. [`work_messages`] drives the blocking
//! receive loop on the calling thread; [`MessageWorker`] exposes the same state
//! for synchronous drivers. [`ApplyBatch`] and [`RoutedInput`] are optional
//! transport containers. [`SnapshotListener`] observes replay affecting watched
//! times; query response traits carry reconstructed snapshots or retained events.
//!
//! Insertion completion does not imply replay completion. Processing starts at
//! the time type's default and [`WorkInputKind::Advance`] raises its target
//! monotonically. Replay processes complete timestamp batches through that target,
//! while queries can reconstruct retained future state without publishing effects.
//! Late changed inputs rewind pending work; duplicates must not do so.
//! Only explicit pruning authorization removes history. [`Coordination`] pauses
//! replay after a fenced report until its round is resolved.

#![deny(missing_docs)]

mod listen;
mod query;
mod types;
mod work;

pub use self::types::{
    AdvanceInput, AdvanceTime, ApplyBatch, ApplyInput, Completion, Coordination, EventInsert, EventQueryInput, EventQueryResponse,
    RouteInput, RoutedInput, SnapshotListenInput, SnapshotListener, SnapshotQueryInput, SnapshotQueryResponse, SnapshotStore, WorkInput,
    WorkInputKind, WorkerCounters, WorkerStatus,
};
pub use self::work::{work_messages, MessageWorker};
