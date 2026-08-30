use std::fmt;
use std::mem::size_of;
use std::ops::Deref;

use crate::types::BoxAllocation;
use crate::{ConservativeTrackedSize, MeasuredAccount, MemoryAccount, MemoryBudget, MemoryKind, TrackedBox};

fn allocation_bytes<T, A, B>(value: &T, account: &A) -> usize
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    account.current(value).saturating_add(size_of::<BoxAllocation<T, A, B>>().saturating_sub(size_of::<T>()))
}

impl<T, B> TrackedBox<T, MeasuredAccount, B>
where
    T: ConservativeTrackedSize,
    B: MemoryBudget,
{
    pub fn new(value: T, budget: B) -> Self {
        Self::new_with_account(value, budget)
    }
}

impl<T, A, B> TrackedBox<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    pub fn new_with_account(value: T, budget: B) -> Self {
        let account = A::new(&value);
        budget.reserve(MemoryKind::Allocation, allocation_bytes::<T, A, B>(&value, &account));
        budget.reserve(MemoryKind::Pointer, size_of::<Self>());
        Self { inner: Box::new(BoxAllocation { value, account, budget }) }
    }

    pub fn update<R>(&mut self, action: impl FnOnce(&mut T) -> R) -> R {
        let inner = &mut *self.inner;
        let (result, change) = inner.account.change(&mut inner.value, action);
        inner.budget.resize(MemoryKind::Allocation, change);
        result
    }
}

impl<T, A, B> Clone for TrackedBox<T, A, B>
where
    T: ConservativeTrackedSize + Clone,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn clone(&self) -> Self {
        Self::new_with_account(self.inner.value.clone(), self.inner.budget.clone())
    }
}

impl<T, A, B> Drop for TrackedBox<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn drop(&mut self) {
        self.inner.budget.release(MemoryKind::Pointer, size_of::<Self>());
    }
}

impl<T, A, B> Drop for BoxAllocation<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn drop(&mut self) {
        self.budget.release(MemoryKind::Allocation, allocation_bytes::<T, A, B>(&self.value, &self.account));
    }
}

impl<T, A, B> Deref for TrackedBox<T, A, B>
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

impl<T, A, B> AsRef<T> for TrackedBox<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T, A, B> fmt::Debug for TrackedBox<T, A, B>
where
    T: ConservativeTrackedSize + fmt::Debug,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<T, A, B> PartialEq for TrackedBox<T, A, B>
where
    T: ConservativeTrackedSize + PartialEq,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl<T, A, B> Eq for TrackedBox<T, A, B>
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

    use crate::{AtomicMemoryBudget, CachedAccount, ConservativeTrackedSize, MemoryBudget, MemoryBudgetConfig, TrackedBox};

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Value(Vec<u8>);

    impl ConservativeTrackedSize for Value {
        fn conservative_tracked_size(&self) -> usize {
            size_of::<Self>() + self.0.capacity()
        }
    }

    fn budget() -> AtomicMemoryBudget {
        AtomicMemoryBudget::new(MemoryBudgetConfig { hard_limit: usize::MAX, concurrent_actions: 0, action_buffer: usize::MAX }).unwrap()
    }

    #[test]
    fn deeply_clones_and_accounts_independent_mutation() {
        let memory = budget();
        let mut original = TrackedBox::new(Value(vec![1]), memory.clone());
        assert_eq!(size_of_val(&original), size_of::<usize>());
        let mut clone = original.clone();
        clone.update(|value| value.0.push(2));
        original.update(|value| value.0.clear());
        assert_eq!(original.0, Vec::<u8>::new());
        assert_eq!(clone.0, vec![1, 2]);
        assert_eq!(format!("{clone:?}"), "Value([1, 2])");
        assert_eq!(memory.state().pointer_bytes, 2 * size_of::<usize>());
        drop(original);
        assert!(memory.state().used > 0);
        drop(clone);
        assert_eq!(memory.state().used, 0);
    }

    #[test]
    fn cached_account_tracks_grow_shrink_and_unchanged_updates() {
        let memory = budget();
        let mut value = TrackedBox::<Value, CachedAccount, _>::new_with_account(Value(Vec::new()), memory.clone());
        let before = memory.state().allocation_bytes;
        assert_eq!(value.update(|_| 7), 7);
        assert_eq!(memory.state().allocation_bytes, before);
        value.update(|value| value.0.reserve_exact(128));
        assert!(memory.state().allocation_bytes >= before + 128);
        value.update(|value| value.0.shrink_to_fit());
        assert_eq!(memory.state().allocation_bytes, before);
        drop(value);
        assert_eq!(memory.state().used, 0);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_tracked_box() {
        let mut criterion = Criterion::default();
        let memory = budget();
        criterion.bench_function("memory/tracked_box/new", |bencher| {
            bencher.iter_batched(
                || (Value(Vec::new()), memory.clone()),
                |(value, memory)| TrackedBox::new(value, memory),
                BatchSize::SmallInput,
            );
        });
        let original = TrackedBox::new(Value(vec![1; 1_000]), memory);
        criterion.bench_function("memory/tracked_box/deep_clone", |bencher| {
            bencher.iter(|| std::hint::black_box(original.clone()));
        });
        criterion.bench_function("memory/tracked_box/drop", |bencher| {
            bencher.iter_batched(|| original.clone(), drop, BatchSize::SmallInput);
        });
        criterion.bench_function("memory/tracked_box/measured_update", |bencher| {
            bencher.iter_batched(|| original.clone(), |mut value| value.update(|value| value.0[0] = 2), BatchSize::SmallInput);
        });
        let cached = TrackedBox::<Value, CachedAccount, _>::new_with_account(Value(vec![1; 1_000]), budget());
        criterion.bench_function("memory/tracked_box/cached_update", |bencher| {
            bencher.iter_batched(|| cached.clone(), |mut value| value.update(|value| value.0[0] = 2), BatchSize::SmallInput);
        });
        criterion.bench_function("memory/tracked_box/vector_growth_1000", |bencher| {
            bencher.iter_batched(
                || TrackedBox::new(Value(Vec::new()), budget()),
                |mut value| value.update(|value| value.0.extend_from_slice(&[0; 1_000])),
                BatchSize::SmallInput,
            );
        });
        criterion.bench_function("memory/tracked_box/deep_clone_drop/1000", |bencher| {
            bencher.iter(|| {
                for _ in 0..1_000 {
                    drop(std::hint::black_box(original.clone()));
                }
            });
        });
        criterion.final_summary();
    }
}
