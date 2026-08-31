use crossbeam_channel::Sender;

/// Constructs the caller-selected message forwarded across the API boundary.
pub trait ApplyOutput<I, R>: Sized {
    fn create(inputs: Vec<I>, rejection_sender: Sender<RejectionMessage<R>>) -> Self;
}

/// One batch of inputs forwarded across the API boundary.
#[derive(Debug)]
pub struct InputBatch<I, R> {
    pub inputs: Vec<I>,
    pub rejection_sender: Sender<RejectionMessage<R>>,
}

impl<I, R> ApplyOutput<I, R> for InputBatch<I, R> {
    fn create(inputs: Vec<I>, rejection_sender: Sender<RejectionMessage<R>>) -> Self {
        Self { inputs, rejection_sender }
    }
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
