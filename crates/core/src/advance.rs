use crossbeam_channel::Sender;

use crate::{ApiError, ConTime, Input, RouterMessage};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    /// Subscribe to the horizon completed by every worker, including retention
    /// hooks. Samples strictly before each reported horizon may be considered
    /// pruned by ConTime. This is not the requested or safe-to-prune target.
    ///
    /// Registration sends the current completed horizon (initially Time::default()),
    /// followed by strictly increasing updates. Each caller has its own unbounded
    /// channel; consumers should drain it regularly. Shutdown closes the channel.
    /// A stopped worker cannot advance the completed horizon by dropping a sender.
    pub fn subscribe_pruned_horizon(&self) -> Result<crossbeam_channel::Receiver<I::Time>, ApiError> {
        let (sender, receiver) = crossbeam_channel::unbounded();
        self.input.send(RouterMessage::SubscribePrunedHorizon(sender)).map_err(|_| ApiError::OutputChannelClosed)?;
        Ok(receiver)
    }

    /// Low-level target submission. Completion acknowledges target delivery,
    /// not completed replay or safe pruning; use `wait_until_idle` for those.
    pub fn send_advance_to(&self, time: I::Time, completion: Sender<()>) -> Result<(), ApiError> {
        contime_api::send_advance_to::<RouterMessage<I, S>, _>(&self.input, time, completion)
    }

    /// Asynchronously raises the processing target and requests retention.
    pub fn advance_to(&self, time: I::Time) -> Result<(), ApiError> {
        let (completion, _) = crossbeam_channel::unbounded();
        self.send_advance_to(time, completion)
    }
}
