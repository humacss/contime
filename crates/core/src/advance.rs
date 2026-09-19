use crossbeam_channel::Sender;

use crate::{ApiError, ConTime, Input, RouterMessage};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
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
