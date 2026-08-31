use std::sync::Arc;

/// A conservative estimate of memory retained by an underlying value.
pub trait ConservativeTrackedSize {
    fn conservative_tracked_size(&self) -> usize;
}

/// A change in conservatively tracked memory usage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SizeDelta {
    Increase(usize),
    Decrease(usize),
    Unchanged,
}

/// Runs a mutation and reports its change in conservatively tracked size.
pub trait TrackedSizeDelta {
    fn size_delta<R>(&mut self, action: impl FnOnce(&mut Self) -> R) -> (R, SizeDelta);
}

/// Receives tracked memory changes and exposes the configured safety buffer.
pub trait TrackedMemoryBudget: Clone {
    fn apply_delta(&self, delta: SizeDelta);
    fn has_buffer(&self) -> bool;
    fn buffer_size(&self) -> usize;
}

pub struct TrackedArc<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    pub(crate) inner: Arc<ArcAllocation<T, B>>,
}

pub(crate) struct ArcAllocation<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    pub(crate) value: T,
    pub(crate) budget: B,
}

pub struct TrackedBox<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    pub(crate) inner: Box<BoxAllocation<T, B>>,
}

pub(crate) struct BoxAllocation<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    pub(crate) value: T,
    pub(crate) budget: B,
}
