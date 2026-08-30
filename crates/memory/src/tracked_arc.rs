use std::fmt;
use std::mem::size_of;
use std::ops::Deref;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crate::types::ArcAllocation;
use crate::{ConservativeTrackedSize, MeasuredAccount, MemoryAccount, MemoryBudget, MemoryKind, TrackedArc};

fn allocation_bytes<T, A, B>(value: &T, account: &A) -> usize
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    account
        .current(value)
        .saturating_add(size_of::<ArcAllocation<T, A, B>>().saturating_sub(size_of::<T>()))
        .saturating_add(2 * size_of::<AtomicUsize>())
}

impl<T, B> TrackedArc<T, MeasuredAccount, B>
where
    T: ConservativeTrackedSize,
    B: MemoryBudget,
{
    pub fn new(value: T, budget: B) -> Self {
        Self::new_with_account(value, budget)
    }
}

impl<T, A, B> TrackedArc<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    pub fn new_with_account(value: T, budget: B) -> Self {
        let account = A::new(&value);
        budget.reserve(MemoryKind::Allocation, allocation_bytes::<T, A, B>(&value, &account));
        budget.reserve(MemoryKind::Pointer, size_of::<Self>());
        Self { inner: Arc::new(ArcAllocation { value, account, budget }) }
    }
}

impl<T, A, B> Clone for TrackedArc<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn clone(&self) -> Self {
        self.inner.budget.reserve(MemoryKind::Pointer, size_of::<Self>());
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<T, A, B> Drop for TrackedArc<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn drop(&mut self) {
        self.inner.budget.release(MemoryKind::Pointer, size_of::<Self>());
    }
}

impl<T, A, B> Drop for ArcAllocation<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn drop(&mut self) {
        self.budget.release(MemoryKind::Allocation, allocation_bytes::<T, A, B>(&self.value, &self.account));
    }
}

impl<T, A, B> Deref for TrackedArc<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner.value
    }
}

impl<T, A, B> AsRef<T> for TrackedArc<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T, A, B> fmt::Debug for TrackedArc<T, A, B>
where
    T: ConservativeTrackedSize + fmt::Debug,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<T, A, B> PartialEq for TrackedArc<T, A, B>
where
    T: ConservativeTrackedSize + PartialEq,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl<T, A, B> Eq for TrackedArc<T, A, B>
where
    T: ConservativeTrackedSize + Eq,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use criterion::{BatchSize, Criterion};

    use crate::{AtomicMemoryBudget, ConservativeTrackedSize, MemoryBudget, MemoryBudgetConfig, TrackedArc};

    #[derive(Debug, Eq, PartialEq)]
    struct Value(u64);

    impl ConservativeTrackedSize for Value {
        fn conservative_tracked_size(&self) -> usize {
            size_of::<Self>()
        }
    }

    fn budget() -> AtomicMemoryBudget {
        AtomicMemoryBudget::new(MemoryBudgetConfig { hard_limit: usize::MAX, concurrent_actions: 0, action_buffer: 0 }).unwrap()
    }

    #[test]
    fn accounts_shared_allocation_and_each_pointer_until_final_drop() {
        let memory = budget();
        let original = TrackedArc::new(Value(7), memory.clone());
        assert_eq!(size_of_val(&original), size_of::<usize>());
        assert_eq!(memory.state().pointer_bytes, size_of::<usize>());
        assert!(memory.state().allocation_bytes >= size_of::<Value>());
        assert_eq!(*original, Value(7));
        assert_eq!(format!("{original:?}"), "Value(7)");

        let clone = original.clone();
        assert!(std::ptr::eq(original.as_ref(), clone.as_ref()));
        assert_eq!(original, clone);
        assert_eq!(memory.state().pointer_bytes, 2 * size_of::<usize>());
        drop(clone);
        assert!(memory.state().allocation_bytes > 0);
        drop(original);
        assert_eq!(memory.state().used, 0);
    }

    #[test]
    fn concurrent_clone_drops_release_every_pointer() {
        let memory = budget();
        let original = TrackedArc::new(Value(7), memory.clone());
        let threads = (0..32)
            .map(|_| {
                let clone = original.clone();
                std::thread::spawn(move || drop(clone))
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(memory.state().pointer_bytes, size_of::<usize>());
        drop(original);
        assert_eq!(memory.state().used, 0);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_tracked_arc() {
        let mut criterion = Criterion::default();
        let memory = budget();
        criterion.bench_function("memory/tracked_arc/new", |bencher| {
            bencher.iter_batched(|| (Value(7), memory.clone()), |(value, memory)| TrackedArc::new(value, memory), BatchSize::SmallInput);
        });
        let original = TrackedArc::new(Value(7), memory);
        criterion.bench_function("memory/tracked_arc/clone", |bencher| {
            bencher.iter(|| std::hint::black_box(original.clone()));
        });
        criterion.bench_function("memory/tracked_arc/drop_non_final", |bencher| {
            bencher.iter_batched(|| original.clone(), drop, BatchSize::SmallInput);
        });
        criterion.bench_function("memory/tracked_arc/drop_final", |bencher| {
            bencher.iter_batched(|| TrackedArc::new(Value(7), budget()), drop, BatchSize::SmallInput);
        });
        criterion.bench_function("memory/tracked_arc/clone_drop/1000", |bencher| {
            bencher.iter(|| {
                for _ in 0..1_000 {
                    drop(std::hint::black_box(original.clone()));
                }
            });
        });
        criterion.final_summary();
    }
}
