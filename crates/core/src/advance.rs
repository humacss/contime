use crossbeam_channel::Sender;

use crate::{ApiError, ConTime, Input, RouterMessage};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    pub fn send_advance_to(&self, time: I::Time, completion: Sender<()>) -> Result<(), ApiError> {
        contime_api::send_advance_to::<RouterMessage<I, S>, _>(self.runtime.input(), time, completion)
    }

    pub fn advance_to(&self, time: I::Time) -> Result<(), ApiError> {
        contime_api::advance_to::<RouterMessage<I, S>, _>(self.runtime.input(), time)
    }
}
