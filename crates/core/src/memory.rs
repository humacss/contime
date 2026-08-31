use std::sync::atomic::Ordering;

use contime_memory::{SizeDelta, TrackedMemoryBudget};

use crate::{types::MemoryState, MemoryBudget};

impl MemoryBudget {
    pub fn new(maximum: usize, buffer: usize) -> Self {
        Self { state: std::sync::Arc::new(MemoryState { used: std::sync::atomic::AtomicUsize::new(0), maximum, buffer }) }
    }

    pub fn used(&self) -> usize {
        self.state.used.load(Ordering::Relaxed)
    }

    pub fn can_admit(&self, bytes: usize) -> bool {
        let usable = self.state.maximum.saturating_sub(self.state.buffer);
        self.used().checked_add(bytes).is_some_and(|total| total <= usable)
    }
}

impl TrackedMemoryBudget for MemoryBudget {
    fn apply_delta(&self, delta: SizeDelta) {
        match delta {
            SizeDelta::Increase(bytes) => {
                let _ = self.state.used.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| Some(used.saturating_add(bytes)));
            }
            SizeDelta::Decrease(bytes) => {
                let _ = self.state.used.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| Some(used.saturating_sub(bytes)));
            }
            SizeDelta::Unchanged => {}
        }
    }

    fn has_buffer(&self) -> bool {
        self.used() <= self.state.maximum.saturating_sub(self.state.buffer)
    }

    fn buffer_size(&self) -> usize {
        self.state.buffer
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use contime_memory::{SizeDelta, TrackedMemoryBudget};
    use criterion::Criterion;

    use crate::MemoryBudget;

    #[test]
    fn budget_applies_deltas_and_reserves_the_configured_buffer() {
        let budget = MemoryBudget::new(1_000, 100);

        assert!(budget.can_admit(900));
        budget.apply_delta(SizeDelta::Increase(700));
        assert_eq!(budget.used(), 700);
        assert!(budget.has_buffer());
        assert!(budget.can_admit(200));
        assert!(!budget.can_admit(201));

        budget.apply_delta(SizeDelta::Decrease(250));
        assert_eq!(budget.used(), 450);
        assert_eq!(budget.buffer_size(), 100);
    }

    #[test]
    fn decreases_saturate_at_zero_and_growth_past_usable_memory_consumes_the_buffer() {
        let budget = MemoryBudget::new(1_000, 100);

        budget.apply_delta(SizeDelta::Decrease(50));
        assert_eq!(budget.used(), 0);

        budget.apply_delta(SizeDelta::Increase(901));
        assert!(!budget.has_buffer());
        assert!(!budget.can_admit(0));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_memory() {
        let mut criterion = Criterion::default();
        let budget = MemoryBudget::new(usize::MAX, 0);
        criterion.bench_function("core/memory/1000_balanced_deltas", |bencher| {
            bencher.iter(|| {
                for _ in 0..1_000 {
                    budget.apply_delta(SizeDelta::Increase(black_box(64)));
                    budget.apply_delta(SizeDelta::Decrease(black_box(64)));
                }
            });
        });
        criterion.final_summary();
    }
}
