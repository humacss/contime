use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

/// A conservative estimate of memory retained by one underlying value.
pub trait ConservativeTrackedSize {
    fn conservative_tracked_size(&self) -> usize;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryChange {
    Increase(usize),
    Decrease(usize),
    Unchanged,
}

pub trait MemoryAccount<T>: Sized
where
    T: ConservativeTrackedSize,
{
    fn new(value: &T) -> Self;
    fn current(&self, value: &T) -> usize;
    fn change<R, F>(&mut self, value: &mut T, action: F) -> (R, MemoryChange)
    where
        F: FnOnce(&mut T) -> R;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    Allocation,
    Pointer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryStatus {
    Ready,
    ActionBlocked,
    HardLimitExceeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryState {
    pub used: usize,
    pub allocation_bytes: usize,
    pub pointer_bytes: usize,
    pub action_ceiling: usize,
    pub hard_limit: usize,
    pub status: MemoryStatus,
    pub buffer_exceeded_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryBudgetConfig {
    pub hard_limit: usize,
    pub concurrent_actions: usize,
    pub action_buffer: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryBudgetConfigError {
    HeadroomOverflow,
    HeadroomExceedsHardLimit,
}

pub trait MemoryBudget: Clone + Send + Sync {
    fn reserve(&self, kind: MemoryKind, bytes: usize);
    fn resize(&self, kind: MemoryKind, change: MemoryChange);
    fn release(&self, kind: MemoryKind, bytes: usize);
    fn state(&self) -> MemoryState;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MeasuredAccount;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CachedAccount {
    pub(crate) bytes: usize,
}

#[derive(Clone, Debug)]
pub struct AtomicMemoryBudget {
    pub(crate) state: Arc<AtomicMemoryState>,
}

#[derive(Debug)]
pub(crate) struct AtomicMemoryState {
    pub(crate) hard_limit: usize,
    pub(crate) action_ceiling: usize,
    pub(crate) action_buffer: usize,
    pub(crate) used: AtomicUsize,
    pub(crate) allocation_bytes: AtomicUsize,
    pub(crate) pointer_bytes: AtomicUsize,
    pub(crate) buffer_exceeded_count: AtomicUsize,
}

pub struct TrackedArc<T, A = MeasuredAccount, B = AtomicMemoryBudget>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    pub(crate) inner: Arc<ArcAllocation<T, A, B>>,
}

pub(crate) struct ArcAllocation<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    pub(crate) value: T,
    pub(crate) account: A,
    pub(crate) budget: B,
}
