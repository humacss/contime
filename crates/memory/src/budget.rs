use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::types::AtomicMemoryState;
use crate::{
    AtomicMemoryBudget, MemoryBudget, MemoryBudgetConfig, MemoryBudgetConfigError, MemoryChange, MemoryKind, MemoryState, MemoryStatus,
};

impl AtomicMemoryBudget {
    pub fn new(config: MemoryBudgetConfig) -> Result<Self, MemoryBudgetConfigError> {
        let headroom = config.concurrent_actions.checked_mul(config.action_buffer).ok_or(MemoryBudgetConfigError::HeadroomOverflow)?;
        let action_ceiling = config.hard_limit.checked_sub(headroom).ok_or(MemoryBudgetConfigError::HeadroomExceedsHardLimit)?;
        Ok(Self {
            state: Arc::new(AtomicMemoryState {
                hard_limit: config.hard_limit,
                action_ceiling,
                action_buffer: config.action_buffer,
                used: AtomicUsize::new(0),
                allocation_bytes: AtomicUsize::new(0),
                pointer_bytes: AtomicUsize::new(0),
                buffer_exceeded_count: AtomicUsize::new(0),
            }),
        })
    }

    fn category(&self, kind: MemoryKind) -> &AtomicUsize {
        match kind {
            MemoryKind::Allocation => &self.state.allocation_bytes,
            MemoryKind::Pointer => &self.state.pointer_bytes,
        }
    }
}

fn add_saturating(counter: &AtomicUsize, bytes: usize) {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| Some(current.saturating_add(bytes)))
        .expect("saturating update always succeeds");
}

fn subtract_checked(counter: &AtomicUsize, bytes: usize) {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| current.checked_sub(bytes))
        .expect("released more memory than reserved");
}

impl MemoryBudget for AtomicMemoryBudget {
    fn reserve(&self, kind: MemoryKind, bytes: usize) {
        add_saturating(self.category(kind), bytes);
        add_saturating(&self.state.used, bytes);
    }

    fn resize(&self, kind: MemoryKind, change: MemoryChange) {
        match change {
            MemoryChange::Increase(bytes) => {
                if bytes > self.state.action_buffer {
                    add_saturating(&self.state.buffer_exceeded_count, 1);
                }
                self.reserve(kind, bytes);
            }
            MemoryChange::Decrease(bytes) => self.release(kind, bytes),
            MemoryChange::Unchanged => {}
        }
    }

    fn release(&self, kind: MemoryKind, bytes: usize) {
        subtract_checked(self.category(kind), bytes);
        subtract_checked(&self.state.used, bytes);
    }

    fn state(&self) -> MemoryState {
        let used = self.state.used.load(Ordering::Acquire);
        let status = if used > self.state.hard_limit {
            MemoryStatus::HardLimitExceeded
        } else if used > self.state.action_ceiling {
            MemoryStatus::ActionBlocked
        } else {
            MemoryStatus::Ready
        };
        MemoryState {
            used,
            allocation_bytes: self.state.allocation_bytes.load(Ordering::Acquire),
            pointer_bytes: self.state.pointer_bytes.load(Ordering::Acquire),
            action_ceiling: self.state.action_ceiling,
            hard_limit: self.state.hard_limit,
            status,
            buffer_exceeded_count: self.state.buffer_exceeded_count.load(Ordering::Acquire),
        }
    }
}

#[cfg(test)]
mod tests {
    use criterion::{BatchSize, Criterion};

    use crate::{AtomicMemoryBudget, MemoryBudget, MemoryBudgetConfig, MemoryBudgetConfigError, MemoryChange, MemoryKind, MemoryStatus};

    fn budget(limit: usize, actions: usize, buffer: usize) -> AtomicMemoryBudget {
        AtomicMemoryBudget::new(MemoryBudgetConfig { hard_limit: limit, concurrent_actions: actions, action_buffer: buffer }).unwrap()
    }

    #[test]
    fn validates_headroom_and_calculates_exact_ceiling() {
        assert_eq!(
            AtomicMemoryBudget::new(MemoryBudgetConfig { hard_limit: usize::MAX, concurrent_actions: usize::MAX, action_buffer: 2 })
                .unwrap_err(),
            MemoryBudgetConfigError::HeadroomOverflow
        );
        assert_eq!(
            AtomicMemoryBudget::new(MemoryBudgetConfig { hard_limit: 9, concurrent_actions: 2, action_buffer: 5 }).unwrap_err(),
            MemoryBudgetConfigError::HeadroomExceedsHardLimit
        );
        assert_eq!(budget(100, 2, 10).state().action_ceiling, 80);
        assert_eq!(budget(100, 0, 10).state().action_ceiling, 100);
    }

    #[test]
    fn accounts_categories_changes_and_thresholds() {
        let memory = budget(100, 1, 20);
        memory.reserve(MemoryKind::Allocation, 70);
        memory.reserve(MemoryKind::Pointer, 8);
        assert_eq!(memory.state().status, MemoryStatus::Ready);
        memory.resize(MemoryKind::Allocation, MemoryChange::Increase(5));
        assert_eq!(memory.state().status, MemoryStatus::ActionBlocked);
        memory.resize(MemoryKind::Allocation, MemoryChange::Decrease(5));
        memory.resize(MemoryKind::Allocation, MemoryChange::Unchanged);
        assert_eq!(memory.state().status, MemoryStatus::Ready);
        memory.reserve(MemoryKind::Allocation, 30);
        let state = memory.state();
        assert_eq!(state.status, MemoryStatus::HardLimitExceeded);
        assert_eq!(state.allocation_bytes, 100);
        assert_eq!(state.pointer_bytes, 8);
        assert_eq!(state.used, 108);
    }

    #[test]
    fn records_large_action_and_saturates_addition() {
        let memory = budget(usize::MAX, 0, 10);
        memory.resize(MemoryKind::Allocation, MemoryChange::Increase(11));
        assert_eq!(memory.state().buffer_exceeded_count, 1);
        memory.reserve(MemoryKind::Allocation, usize::MAX);
        assert_eq!(memory.state().used, usize::MAX);
    }

    #[test]
    fn concurrent_balanced_accounting_returns_to_zero() {
        let memory = budget(usize::MAX, 0, 0);
        let threads = (0..8)
            .map(|_| {
                let memory = memory.clone();
                std::thread::spawn(move || {
                    for _ in 0..10_000 {
                        memory.reserve(MemoryKind::Pointer, 1);
                        memory.release(MemoryKind::Pointer, 1);
                    }
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(memory.state().used, 0);
    }

    #[test]
    #[should_panic(expected = "released more")]
    fn release_underflow_panics() {
        budget(100, 0, 0).release(MemoryKind::Pointer, 1);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_budget() {
        let mut criterion = Criterion::default();
        let allocation = budget(usize::MAX, 0, usize::MAX);
        criterion.bench_function("memory/budget/reserve_allocation/1000", |bencher| {
            bencher.iter(|| {
                for _ in 0..1_000 {
                    allocation.reserve(MemoryKind::Allocation, 8);
                }
            });
        });
        let pointer = budget(usize::MAX, 0, usize::MAX);
        criterion.bench_function("memory/budget/reserve_pointer/1000", |bencher| {
            bencher.iter(|| {
                for _ in 0..1_000 {
                    pointer.reserve(MemoryKind::Pointer, 8);
                }
            });
        });
        let increase = budget(usize::MAX, 0, usize::MAX);
        criterion.bench_function("memory/budget/resize_increase/1000", |bencher| {
            bencher.iter(|| {
                for _ in 0..1_000 {
                    increase.resize(MemoryKind::Allocation, MemoryChange::Increase(8));
                }
            });
        });
        criterion.bench_function("memory/budget/resize_decrease/1000", |bencher| {
            bencher.iter_batched(
                || {
                    let memory = budget(usize::MAX, 0, usize::MAX);
                    memory.reserve(MemoryKind::Allocation, 8_000);
                    memory
                },
                |memory| {
                    for _ in 0..1_000 {
                        memory.resize(MemoryKind::Allocation, MemoryChange::Decrease(8));
                    }
                },
                BatchSize::SmallInput,
            );
        });
        let balanced = budget(usize::MAX, 0, usize::MAX);
        criterion.bench_function("memory/budget/balanced/1000", |bencher| {
            bencher.iter(|| {
                for _ in 0..1_000 {
                    balanced.reserve(MemoryKind::Pointer, 8);
                    balanced.release(MemoryKind::Pointer, 8);
                }
            });
        });
        criterion.final_summary();
    }
}
