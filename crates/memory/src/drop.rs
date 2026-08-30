use std::mem::size_of;

use crate::types::{Allocation, MemoryAccount, MemoryKind, TrackedArc};

impl<T, M> Drop for TrackedArc<T, M>
where
    M: MemoryAccount,
{
    fn drop(&mut self) {
        self.inner.memory.release(MemoryKind::Pointer, size_of::<Self>() as u64);
    }
}

impl<T, M> Drop for Allocation<T, M>
where
    M: MemoryAccount,
{
    fn drop(&mut self) {
        self.memory.release(MemoryKind::Allocation, self.allocation_bytes);
    }
}
