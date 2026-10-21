use crossbeam_channel::Sender;

use crate::{ApiError, ConTime, Input, SnapshotListenerMessage};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    pub fn send_listen_snapshots(
        &self,
        time: I::Time,
        snapshot_ids: impl IntoIterator<Item = u128>,
        notifications: Sender<SnapshotListenerMessage<I::Time>>,
    ) -> Result<(), ApiError> {
        contime_api::send_listen_snapshots(&self.input, time, snapshot_ids, notifications)
    }
}
