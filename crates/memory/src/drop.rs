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

#[cfg(test)]
mod tests {
    use criterion::{BatchSize, Criterion};

    use crate::{ConservativeSize, MemoryBudget, TrackedArc};

    struct Value;

    impl ConservativeSize for Value {
        fn conservative_size(&self) -> u64 {
            64
        }
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_drop() {
        let mut criterion = Criterion::default();

        criterion.bench_function("memory/tracked_arc/drop_non_final", |bencher| {
            bencher.iter_batched(
                || {
                    let original = TrackedArc::try_new(Value, MemoryBudget::new(1_000)).unwrap();
                    let clone = original.try_clone().unwrap();
                    (original, clone)
                },
                |(original, clone)| {
                    drop(clone);
                    std::hint::black_box(original)
                },
                BatchSize::SmallInput,
            );
        });

        criterion.bench_function("memory/tracked_arc/drop_final", |bencher| {
            bencher.iter_batched(|| TrackedArc::try_new(Value, MemoryBudget::new(1_000)).unwrap(), drop, BatchSize::SmallInput);
        });

        criterion.final_summary();
    }
}
