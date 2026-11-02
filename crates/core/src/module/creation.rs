//! Snapshot initialization is independent of timestamp-batch application.
use super::{EventRef, SnapshotEvents};

/// Initializes a targeted snapshot from its first registered event.
///
/// Derives can copy declared identity fields and default the remaining state.
/// Implement manually for fanout, custom initialization or non-default state.
pub trait SnapshotCreate: SnapshotEvents {
    /// Initializes an identity before application; the returned ID must match it.
    fn create(snapshot_id: u128, event: EventRef<'_>) -> Self;
}
