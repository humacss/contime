//! Snapshot history accessed through [`SnapshotStore`].
//! Consumers implement [`Snapshot`], [`Event`], [`EventStore`], and [`Apply`].
//! Forwarding additionally requires [`Timestamp`].

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
pub use types::{Apply, Checkpoint, Event, EventStore, NoCheckpoint, Snapshot, Timestamp};

pub(crate) use commit::Commit;
pub(crate) use playback::Playback;
pub(crate) use store::Store;
