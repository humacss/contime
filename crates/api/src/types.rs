use crossbeam_channel::Sender;

/// Constructs the caller-selected message forwarded across the API boundary.
pub trait ApplyOutput<I, R>: Sized {
    /// Builds one request owning the inputs and its rejection channel.
    ///
    /// Keep the sender, or its clones, alive until downstream insertion handling
    /// finishes. Dropping all copies closes the request's rejection stream.
    fn create(inputs: Vec<I>, rejection_sender: Sender<RejectionMessage<R>>) -> Self;
}

/// Constructs a caller-selected snapshot-query message.
pub trait SnapshotQueryOutput<T, S>: Sized {
    /// Builds a query for the requested IDs at `time` with a result-batch sender.
    ///
    /// Downstream code sends found snapshots and drops every sender clone when
    /// finished; the API does not enforce result order or timestamp semantics.
    fn snapshot_query(time: T, snapshot_ids: Vec<u128>, response: Sender<Vec<Box<S>>>) -> Self;
}

/// Constructs a caller-selected snapshot-listener registration message.
pub trait SnapshotListenOutput<T, N>: Sized {
    /// Builds a registration for `snapshot_ids` at the watched `time`.
    ///
    /// The downstream adapter defines notification contents and when registration
    /// takes effect. The caller owns the receiving side of `notifications`.
    fn listen(time: T, snapshot_ids: Vec<u128>, notifications: Sender<N>) -> Self;
}

/// Constructs a caller-selected event-history query message.
pub trait EventQueryOutput<T, E>: Sized {
    /// Builds a query for one history over the half-open interval `[from, to)`.
    ///
    /// The API forwards bounds unchanged. Downstream code owns validation,
    /// result ordering and release of all response sender clones.
    fn event_query(snapshot_id: u128, from: T, to: T, response: Sender<Vec<E>>) -> Self;
}

/// Constructs a caller-selected horizon-advancement message.
pub trait AdvanceOutput<T>: Sized {
    /// Builds an advancement request carrying its completion sender.
    ///
    /// Release every sender clone when the adapter's advancement boundary is
    /// complete. Synchronous API callers wait for closure, not a unit payload.
    fn advance(time: T, completion: Sender<()>) -> Self;
}

/// One batch of inputs forwarded across the API boundary.
#[derive(Debug)]
pub struct InputBatch<I, R> {
    /// Inputs in the order supplied to the forwarding helper.
    pub inputs: Vec<I>,
    /// Reports rejected inputs and keeps this request open while retained.
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
    /// Identity of the input rejected by the downstream adapter.
    pub event_id: u128,
    /// Adapter-defined reason the input could not be inserted.
    pub reason: R,
}

/// Failure to forward an API message to its downstream receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// The downstream receiver disconnected before the request was queued.
    OutputChannelClosed,
}

/// Rejections returned after every sender associated with an apply has closed.
pub type ApplyResponse<R> = Vec<RejectionMessage<R>>;
