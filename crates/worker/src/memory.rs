#[derive(Debug)]
pub(crate) struct Memory {
    limit: u64,
    used: u64,
}

impl Memory {
    pub(crate) const fn new(limit: u64) -> Self {
        Self { limit, used: 0 }
    }

    #[cfg(test)]
    pub(crate) const fn used(&self) -> u64 {
        self.used
    }

    pub(crate) const fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }

    pub(crate) fn try_reserve(&mut self, bytes: u64) -> bool {
        let Some(next) = self.used.checked_add(bytes) else {
            return false;
        };
        if next > self.limit {
            return false;
        }
        self.used = next;
        true
    }

    pub(crate) fn retained_limit_for(&self, reserved_bytes: u64) -> u64 {
        reserved_bytes.saturating_add(self.remaining())
    }

    pub(crate) fn reconcile(&mut self, reserved_bytes: u64, retained_bytes_delta: i64) {
        self.used = self.used.saturating_sub(reserved_bytes);

        self.apply_delta(retained_bytes_delta);
    }

    pub(crate) fn apply_delta(&mut self, retained_bytes_delta: i64) {
        if retained_bytes_delta >= 0 {
            let retained_bytes = retained_bytes_delta as u64;
            assert!(retained_bytes <= self.remaining(), "history or replay exceeded its worker-provided retained-memory limit");
            self.used += retained_bytes;
        } else {
            self.used = self.used.saturating_sub(retained_bytes_delta.unsigned_abs());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::{BatchSize, Criterion};

    use super::Memory;

    #[test]
    fn reservation_and_reconciliation_keep_only_the_reported_delta() {
        let mut memory = Memory::new(100);

        assert!(memory.try_reserve(60));
        memory.reconcile(60, 25);

        assert_eq!(memory.used(), 25);
        assert_eq!(memory.remaining(), 75);
    }

    #[test]
    fn a_complete_reservation_is_rejected_without_changing_usage() {
        let mut memory = Memory::new(100);

        assert!(!memory.try_reserve(101));

        assert_eq!(memory.used(), 0);
    }

    #[test]
    fn replay_delta_uses_the_same_worker_budget() {
        let mut memory = Memory::new(100);
        memory.apply_delta(25);
        memory.apply_delta(-10);

        assert_eq!(memory.used(), 15);
        assert_eq!(memory.remaining(), 85);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_memory() {
        let mut criterion = Criterion::default();

        criterion.bench_function("worker/memory/1000_reserve_reconcile", |bencher| {
            bencher.iter_batched(
                || Memory::new(128_000),
                |mut memory| {
                    for _ in 0..1_000 {
                        black_box(memory.try_reserve(64));
                        memory.reconcile(64, 64);
                    }
                    black_box(memory.used())
                },
                BatchSize::SmallInput,
            );
        });

        criterion.final_summary();
    }
}
