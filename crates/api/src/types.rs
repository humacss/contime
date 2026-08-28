use std::sync::Arc;

use contime::{EventRejection, EventRejectionReason};
use crossbeam_channel::Sender;

/// One batch of inputs forwarded across the API boundary.
#[derive(Debug)]
pub struct InputBatch<I> {
    pub inputs: Vec<Arc<I>>,
    pub rejection_sender: Sender<RejectionMessage>,
}

/// One rejected input returned across the API boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectionMessage {
    pub event_id: u128,
    pub reason: EventRejectionReason,
}

/// Failure to forward an API message to its downstream receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiError {
    OutputChannelClosed,
}

/// Rejections returned after every sender associated with an apply has closed.
pub type ApplyResponse = Vec<EventRejection>;
