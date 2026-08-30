use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::types::{BudgetState, MemoryAccount, MemoryBudget, MemoryFull, MemoryKind};

impl MemoryBudget {
    pub fn new(limit: u64) -> Self {
        Self { state: Arc::new(BudgetState { limit, used: 0.into(), allocation_bytes: 0.into(), pointer_bytes: 0.into() }) }
    }

    pub fn limit(&self) -> u64 {
        self.state.limit
    }

    pub fn used(&self) -> u64 {
        self.state.used.load(Ordering::Acquire)
    }

    pub fn remaining(&self) -> u64 {
        self.limit().saturating_sub(self.used())
    }

    pub fn allocation_bytes(&self) -> u64 {
        self.state.allocation_bytes.load(Ordering::Acquire)
    }

    pub fn pointer_bytes(&self) -> u64 {
        self.state.pointer_bytes.load(Ordering::Acquire)
    }

    fn category(&self, kind: MemoryKind) -> &std::sync::atomic::AtomicU64 {
        match kind {
            MemoryKind::Allocation => &self.state.allocation_bytes,
            MemoryKind::Pointer => &self.state.pointer_bytes,
        }
    }
}

impl MemoryAccount for MemoryBudget {
    type Error = MemoryFull;

    fn try_reserve(&self, kind: MemoryKind, bytes: u64) -> Result<(), Self::Error> {
        let mut current = self.state.used.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(bytes).filter(|next| *next <= self.state.limit) else {
                return Err(MemoryFull { requested: bytes, remaining: self.state.limit.saturating_sub(current) });
            };
            match self.state.used.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    self.category(kind).fetch_add(bytes, Ordering::Release);
                    return Ok(());
                }
                Err(observed) => current = observed,
            }
        }
    }

    fn release(&self, kind: MemoryKind, bytes: u64) {
        let category_before = self.category(kind).fetch_sub(bytes, Ordering::AcqRel);
        let total_before = self.state.used.fetch_sub(bytes, Ordering::AcqRel);
        assert!(category_before >= bytes, "released more category memory than reserved");
        assert!(total_before >= bytes, "released more total memory than reserved");
    }
}

#[cfg(test)]
mod tests {
    use crate::{MemoryAccount, MemoryBudget, MemoryFull, MemoryKind};

    #[test]
    fn reservations_share_one_limit_and_preserve_categories() {
        let memory = MemoryBudget::new(64);

        memory.try_reserve(MemoryKind::Allocation, 40).unwrap();
        memory.try_reserve(MemoryKind::Pointer, 8).unwrap();

        assert_eq!(memory.limit(), 64);
        assert_eq!(memory.used(), 48);
        assert_eq!(memory.remaining(), 16);
        assert_eq!(memory.allocation_bytes(), 40);
        assert_eq!(memory.pointer_bytes(), 8);
    }

    #[test]
    fn failed_reservation_changes_no_accounting_state() {
        let memory = MemoryBudget::new(16);
        memory.try_reserve(MemoryKind::Allocation, 12).unwrap();

        let error = memory.try_reserve(MemoryKind::Pointer, 8).unwrap_err();

        assert_eq!(error, MemoryFull { requested: 8, remaining: 4 });
        assert_eq!(memory.used(), 12);
        assert_eq!(memory.allocation_bytes(), 12);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn release_returns_category_and_total_bytes() {
        let memory = MemoryBudget::new(64);
        memory.try_reserve(MemoryKind::Allocation, 40).unwrap();
        memory.try_reserve(MemoryKind::Pointer, 8).unwrap();

        memory.release(MemoryKind::Pointer, 8);
        memory.release(MemoryKind::Allocation, 40);

        assert_eq!(memory.used(), 0);
        assert_eq!(memory.remaining(), 64);
        assert_eq!(memory.allocation_bytes(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn cloned_budgets_share_state() {
        let memory = MemoryBudget::new(64);
        let clone = memory.clone();

        clone.try_reserve(MemoryKind::Allocation, 24).unwrap();

        assert_eq!(memory.used(), 24);
        memory.release(MemoryKind::Allocation, 24);
        assert_eq!(clone.used(), 0);
    }

    #[test]
    fn overflow_cannot_bypass_the_limit() {
        let memory = MemoryBudget::new(u64::MAX);
        memory.try_reserve(MemoryKind::Allocation, u64::MAX).unwrap();

        assert!(memory.try_reserve(MemoryKind::Pointer, 1).is_err());
        assert_eq!(memory.used(), u64::MAX);
    }
}
