//! Bounded packetization and nonblocking queue draining.
/// Default maximum number of items in a transport packet.
pub const MAX_EVENTS: usize = 1_024;
/// Maximum original requests gathered during one drain, including empty requests.
pub const MAX_REQUESTS: usize = MAX_EVENTS;
/// Default soft gathering-work budget used by ConTime's coordinator.
///
/// This is not a fill delay. The coordinator checks its clock after each request;
/// a single admission callback or large request can exceed the budget.
pub const GATHER_WORK: std::time::Duration = std::time::Duration::from_micros(50);

/// An owned input segment and the context that must travel with its items.
///
/// Context clones must retain any completion ownership until every split has
/// finished. Compatibility must mean merging preserves response ownership and
/// destination; segments with distinct request responses must remain separate.
pub trait Batch: Sized {
    /// One transported item. Items need not implement `Clone`.
    type Item;
    /// Context shared by chunks of a split submission.
    type Context: Clone;
    /// Consume a segment without cloning its items.
    fn into_parts(self) -> (Vec<Self::Item>, Self::Context);
    /// Construct a segment for a chunk and its retained context.
    fn from_parts(items: Vec<Self::Item>, context: Self::Context) -> Self;
    /// Borrow the owned item vector for length checks and adjacent merging.
    fn items(&mut self) -> &mut Vec<Self::Item>;
    /// Whether adjacent segments may safely share the first segment's context.
    fn can_merge(&self, other: &Self) -> bool;
}

/// A queued message that may be batchable input or an ordering boundary.
pub trait Request: Sized {
    /// Batchable input carried by this message.
    type Batch: Batch;
    /// Extract input, or return the untouched control message as a boundary.
    fn into_batch(self) -> Result<Self::Batch, Self>;
}

/// Builds bounded packets while retaining distinct response segments.
///
/// Full packets are dispatched immediately. Call [`Self::flush`] when no more
/// ready input is available or before forwarding a control message. Dropping a
/// batcher releases buffered ownership; it does not dispatch buffered work.
/// Dispatch failure can follow successful earlier packets and provides no rollback.
pub struct Batcher<B: Batch, F> {
    emit: F,
    segments: Vec<B>,
    events: usize,
    limit: usize,
}
impl<B: Batch, F, E> Batcher<B, F>
where
    F: FnMut(Vec<B>) -> Result<(), E>,
{
    /// Create a batcher with a dispatch callback and positive item limit.
    ///
    /// # Panics
    /// Panics if `limit` is zero.
    pub fn new(emit: F, limit: usize) -> Self {
        assert!(limit > 0, "packet limit must be positive");
        Self { emit, segments: Vec::new(), events: 0, limit }
    }
    /// Move a submission into packets, splitting as needed without cloning items.
    /// Empty submissions release their context immediately.
    pub fn push(&mut self, mut batch: B) -> Result<(), E> {
        let len = batch.items().len();
        if len == 0 {
            return Ok(());
        }
        if len <= self.limit - self.events {
            self.events += len;
            if let Some(last) = self.segments.last_mut().filter(|last| last.can_merge(&batch)) {
                last.items().append(batch.items());
            } else {
                self.segments.push(batch);
            }
            if self.events == self.limit {
                self.flush()?;
            }
            return Ok(());
        }
        let (inputs, context) = batch.into_parts();
        let mut inputs = inputs.into_iter();
        while !inputs.as_slice().is_empty() {
            let count = inputs.len().min(self.limit - self.events);
            let chunk = inputs.by_ref().take(count).collect();
            self.push(B::from_parts(chunk, context.clone()))?;
        }
        Ok(())
    }
    /// Dispatch pending work immediately. Empty packets are never dispatched.
    pub fn flush(&mut self) -> Result<(), E> {
        let segments = std::mem::take(&mut self.segments);
        self.events = 0;
        if segments.is_empty() {
            return Ok(());
        }
        (self.emit)(segments)
    }
}

/// Gather ready requests into bounded packets, never crossing a control message.
///
/// `next_ready` must be nonblocking and return `None` immediately when no request
/// is ready. Each original submission passes through `admit` before packetization;
/// returning `None` rejects or discards it. `may_gather` is checked after each
/// request and can impose a soft time budget without deliberately waiting.
///
/// Always flushes before a successful return, including an empty queue, exhausted
/// budget, request-count limit, or control boundary. A returned boundary has been
/// removed from the queue; the caller must handle it before reading further.
/// Dispatch errors propagate unchanged and do not roll back prior dispatches.
pub fn drain<M, N, F, A, G, E>(first: M::Batch, mut next_ready: N, emit: F, mut admit: A, mut may_gather: G) -> Result<Option<M>, E>
where
    M: Request,
    N: FnMut() -> Option<M>,
    F: FnMut(Vec<M::Batch>) -> Result<(), E>,
    A: FnMut(M::Batch) -> Option<M::Batch>,
    G: FnMut() -> bool,
{
    let mut batcher = Batcher::new(emit, MAX_EVENTS);
    let mut next = Some(first);
    let mut boundary = None;
    for request in 0..MAX_REQUESTS {
        if let Some(batch) = admit(next.take().unwrap()) {
            batcher.push(batch)?;
        }
        if request + 1 == MAX_REQUESTS || !may_gather() {
            break;
        }
        match next_ready() {
            Some(message) => match message.into_batch() {
                Ok(batch) => next = Some(batch),
                Err(other) => {
                    boundary = Some(other);
                    break;
                }
            },
            None => break,
        }
    }
    batcher.flush()?;
    Ok(boundary)
}
