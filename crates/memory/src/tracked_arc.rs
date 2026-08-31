use std::fmt;
use std::mem::size_of;
use std::ops::Deref;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crate::types::ArcAllocation;
use crate::{ConservativeTrackedSize, SizeDelta, TrackedArc, TrackedMemoryBudget};

fn allocation_size<T, B>(value: &T) -> usize
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    value
        .conservative_tracked_size()
        .saturating_add(size_of::<ArcAllocation<T, B>>().saturating_sub(size_of::<T>()))
        .saturating_add(2 * size_of::<AtomicUsize>())
}

impl<T, B> TrackedArc<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    pub fn new(value: T, budget: B) -> Self {
        let allocation = allocation_size::<T, B>(&value);
        let pointer = size_of::<Self>();
        budget.apply_delta(SizeDelta::Increase(allocation.saturating_add(pointer)));
        Self { inner: Arc::new(ArcAllocation { value, budget }) }
    }
}

impl<T, B> Clone for TrackedArc<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    fn clone(&self) -> Self {
        self.inner.budget.apply_delta(SizeDelta::Increase(size_of::<Self>()));
        Self { inner: Arc::clone(&self.inner) }
    }
}

impl<T, B> Drop for TrackedArc<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    fn drop(&mut self) {
        self.inner.budget.apply_delta(SizeDelta::Decrease(size_of::<Self>()));
    }
}

impl<T, B> Drop for ArcAllocation<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    fn drop(&mut self) {
        self.budget.apply_delta(SizeDelta::Decrease(allocation_size::<T, B>(&self.value)));
    }
}

impl<T, B> Deref for TrackedArc<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner.value
    }
}

impl<T, B> AsRef<T> for TrackedArc<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T, B> fmt::Debug for TrackedArc<T, B>
where
    T: ConservativeTrackedSize + fmt::Debug,
    B: TrackedMemoryBudget,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<T, B> PartialEq for TrackedArc<T, B>
where
    T: ConservativeTrackedSize + PartialEq,
    B: TrackedMemoryBudget,
{
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl<T, B> Eq for TrackedArc<T, B>
where
    T: ConservativeTrackedSize + Eq,
    B: TrackedMemoryBudget,
{
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{Arc, Mutex};

    use criterion::{BatchSize, Criterion};

    use crate::{ConservativeTrackedSize, SizeDelta, TrackedArc, TrackedMemoryBudget};

    #[derive(Clone, Default)]
    struct RecordingBudget {
        deltas: Arc<Mutex<Vec<SizeDelta>>>,
    }

    impl RecordingBudget {
        fn deltas(&self) -> Vec<SizeDelta> {
            self.deltas.lock().unwrap().clone()
        }
    }

    impl TrackedMemoryBudget for RecordingBudget {
        fn apply_delta(&self, delta: SizeDelta) {
            self.deltas.lock().unwrap().push(delta);
        }

        fn has_buffer(&self) -> bool {
            true
        }

        fn buffer_size(&self) -> usize {
            usize::MAX
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    struct Value(u64);

    impl ConservativeTrackedSize for Value {
        fn conservative_tracked_size(&self) -> usize {
            size_of::<Self>()
        }
    }

    #[derive(Clone, Default)]
    struct BenchmarkBudget {
        bytes: Arc<AtomicIsize>,
    }

    impl TrackedMemoryBudget for BenchmarkBudget {
        fn apply_delta(&self, delta: SizeDelta) {
            let change = match delta {
                SizeDelta::Increase(bytes) => bytes as isize,
                SizeDelta::Decrease(bytes) => -(bytes as isize),
                SizeDelta::Unchanged => 0,
            };
            self.bytes.fetch_add(change, Ordering::Relaxed);
        }

        fn has_buffer(&self) -> bool {
            true
        }

        fn buffer_size(&self) -> usize {
            usize::MAX
        }
    }

    #[test]
    fn accounts_each_handle_and_releases_the_shared_allocation_once() {
        let budget = RecordingBudget::default();
        let pointer = size_of::<TrackedArc<Value, RecordingBudget>>();
        let allocation = size_of::<Value>() + size_of::<RecordingBudget>() + 2 * size_of::<std::sync::atomic::AtomicUsize>();

        let original = TrackedArc::new(Value(7), budget.clone());
        assert_eq!(size_of_val(&original), size_of::<usize>());
        assert_eq!(*original, Value(7));

        let clone = original.clone();
        assert!(std::ptr::eq(original.as_ref(), clone.as_ref()));
        drop(clone);
        assert_eq!(
            budget.deltas(),
            vec![SizeDelta::Increase(allocation + pointer), SizeDelta::Increase(pointer), SizeDelta::Decrease(pointer),]
        );

        drop(original);
        assert_eq!(
            budget.deltas(),
            vec![
                SizeDelta::Increase(allocation + pointer),
                SizeDelta::Increase(pointer),
                SizeDelta::Decrease(pointer),
                SizeDelta::Decrease(pointer),
                SizeDelta::Decrease(allocation),
            ]
        );
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_tracked_arc() {
        let mut criterion = Criterion::default();
        let original = TrackedArc::new(Value(7), BenchmarkBudget::default());
        criterion.bench_function("memory/unit/tracked_arc/clone", |bencher| {
            bencher.iter_batched(|| (), |_| std::hint::black_box(original.clone()), BatchSize::SmallInput);
        });
        criterion.final_summary();
    }
}
