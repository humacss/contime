//! Register caller-owned channels for snapshot replay notifications.

use crossbeam_channel::Sender;

use crate::contime_runtime::{ApiError, ConTime, SnapshotListenerMessage};

impl<I, S, W> ConTime<I, S, W>
where
    I: crate::contime_runtime::ThreadInput,
{
    /// Registers replay notifications for the requested IDs through `time`.
    ///
    /// [`SnapshotListenerMessage::Registered`] acknowledges registration, followed
    /// by replay notifications while the listener remains relevant. Notifications
    /// concern replay, not durable delivery or a transaction boundary. Drain the
    /// channel concurrently if it is bounded; dropping its receiver unsubscribes.
    /// Returns [`ApiError`] if the registration cannot be enqueued.
    pub fn send_listen_snapshots(
        &self,
        time: I::Time,
        snapshot_ids: impl IntoIterator<Item = u128>,
        notifications: Sender<SnapshotListenerMessage<I::Time>>,
    ) -> Result<(), ApiError> {
        crate::contime_api::send_listen_snapshots(&self.input, time, snapshot_ids, notifications)
    }
}
