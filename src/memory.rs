use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct MemoryTracker {
    budget: Arc<AtomicU64>,
    usage: Arc<AtomicU64>,
}

impl MemoryTracker {
    pub(crate) fn new(budget_bytes: u64) -> Self {
        Self { budget: Arc::new(AtomicU64::new(budget_bytes)), usage: Arc::new(AtomicU64::new(0)) }
    }

    pub(crate) fn remaining(&self) -> u64 {
        self.budget.load(Ordering::Relaxed).saturating_sub(self.usage.load(Ordering::Relaxed))
    }

    pub(crate) fn can_fit(&self, bytes: u64) -> bool {
        bytes <= self.remaining()
    }

    pub(crate) fn try_reserve(&self, bytes: u64) -> bool {
        let budget = self.budget.load(Ordering::Relaxed);
        self.usage.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(bytes).filter(|next| *next <= budget)).is_ok()
    }

    pub(crate) fn release(&self, bytes: u64) {
        self.usage.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| Some(used.saturating_sub(bytes))).ok();
    }

    pub(crate) fn apply_delta(&self, delta: i64) {
        self.usage
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                Some(if delta >= 0 { used.saturating_add(delta as u64) } else { used.saturating_sub(delta.unsigned_abs()) })
            })
            .ok();
    }

    pub(crate) fn reconcile_reservation(&self, reserved: u64, actual_delta: i64) {
        if actual_delta >= 0 {
            let actual_growth = actual_delta as u64;
            debug_assert!(
                actual_growth <= reserved,
                "actual memory growth ({actual_growth} bytes) exceeded its conservative reservation ({reserved} bytes)"
            );
            self.release(reserved.saturating_sub(actual_growth));
        } else {
            self.release(reserved);
            self.apply_delta(actual_delta);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryTracker;

    #[test]
    fn advisory_check_does_not_reserve_memory() {
        let tracker = MemoryTracker::new(100);
        assert!(tracker.can_fit(80));
        assert_eq!(tracker.remaining(), 100);
    }

    #[test]
    fn whole_message_reservation_is_atomic() {
        let tracker = MemoryTracker::new(100);
        assert!(tracker.try_reserve(80));
        assert!(!tracker.try_reserve(21));
        assert_eq!(tracker.remaining(), 20);
    }

    #[test]
    fn releasing_overestimate_restores_capacity() {
        let tracker = MemoryTracker::new(100);
        assert!(tracker.try_reserve(80));
        tracker.release(30);
        assert_eq!(tracker.remaining(), 50);
    }

    #[test]
    fn reservation_reconciliation_keeps_only_actual_growth() {
        let tracker = MemoryTracker::new(100);
        assert!(tracker.try_reserve(80));
        tracker.reconcile_reservation(80, 30);
        assert_eq!(tracker.remaining(), 70);
    }

    #[test]
    fn negative_actual_delta_releases_reservation_and_existing_usage() {
        let tracker = MemoryTracker::new(100);
        assert!(tracker.try_reserve(40));
        assert!(tracker.try_reserve(20));
        tracker.reconcile_reservation(20, -10);
        assert_eq!(tracker.remaining(), 70);
    }
}
