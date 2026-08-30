use std::sync::atomic::AtomicU64;
use std::sync::Arc;

/// A conservative retained-memory estimate for one complete value graph.
pub trait ConservativeSize {
    fn conservative_size(&self) -> u64;
}

/// The category charged by one memory-accounting operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    Allocation,
    Pointer,
}

/// Thread-safe retained-memory reservation and release.
pub trait MemoryAccount: Clone + Send + Sync {
    type Error;

    fn try_reserve(&self, kind: MemoryKind, bytes: u64) -> Result<(), Self::Error>;
    fn release(&self, kind: MemoryKind, bytes: u64);
}

/// A reservation that would exceed the shared memory limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryFull {
    pub requested: u64,
    pub remaining: u64,
}

/// One cloneable handle to a shared atomic memory budget.
#[derive(Clone)]
pub struct MemoryBudget {
    pub(crate) state: Arc<BudgetState>,
}

pub(crate) struct BudgetState {
    pub(crate) limit: u64,
    pub(crate) used: AtomicU64,
    pub(crate) allocation_bytes: AtomicU64,
    pub(crate) pointer_bytes: AtomicU64,
}
