use crossbeam_channel::Sender;

/// One batch of inputs forwarded across the API boundary.
#[derive(Debug)]
pub struct InputBatch<I, R> {
    pub inputs: Vec<I>,
    pub rejection_sender: Sender<RejectionMessage<R>>,
}

/// One rejected input returned across the API boundary.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RejectionMessage<R> {
    pub event_id: u128,
    pub reason: R,
}

/// Failure to forward an API message to its downstream receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiError {
    OutputChannelClosed,
}

/// Rejections returned after every sender associated with an apply has closed.
pub type ApplyResponse<R> = Vec<RejectionMessage<R>>;
