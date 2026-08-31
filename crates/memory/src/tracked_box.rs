use std::fmt;
use std::mem::size_of;
use std::ops::Deref;

use crate::types::BoxAllocation;
use crate::{ConservativeTrackedSize, SizeDelta, TrackedBox, TrackedMemoryBudget, TrackedSizeDelta};

fn allocation_size<T, B>(value: &T) -> usize
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    value.conservative_tracked_size().saturating_add(size_of::<BoxAllocation<T, B>>().saturating_sub(size_of::<T>()))
}

impl<T, B> TrackedBox<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    pub fn new(value: T, budget: B) -> Self {
        let allocation = allocation_size::<T, B>(&value);
        let pointer = size_of::<Self>();
        budget.apply_delta(SizeDelta::Increase(allocation.saturating_add(pointer)));
        Self { inner: Box::new(BoxAllocation { value, budget }) }
    }

    pub fn update<R>(&mut self, action: impl FnOnce(&mut T) -> R) -> R
    where
        T: TrackedSizeDelta,
    {
        let inner = &mut *self.inner;
        let (result, delta) = inner.value.size_delta(action);
        inner.budget.apply_delta(delta);
        result
    }
}

impl<T, B> Clone for TrackedBox<T, B>
where
    T: ConservativeTrackedSize + Clone,
    B: TrackedMemoryBudget,
{
    fn clone(&self) -> Self {
        Self::new(self.inner.value.clone(), self.inner.budget.clone())
    }
}

impl<T, B> Drop for TrackedBox<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    fn drop(&mut self) {
        self.inner.budget.apply_delta(SizeDelta::Decrease(size_of::<Self>()));
    }
}

impl<T, B> Drop for BoxAllocation<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    fn drop(&mut self) {
        self.budget.apply_delta(SizeDelta::Decrease(allocation_size::<T, B>(&self.value)));
    }
}

impl<T, B> Deref for TrackedBox<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner.value
    }
}

impl<T, B> AsRef<T> for TrackedBox<T, B>
where
    T: ConservativeTrackedSize,
    B: TrackedMemoryBudget,
{
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T, B> fmt::Debug for TrackedBox<T, B>
where
    T: ConservativeTrackedSize + fmt::Debug,
    B: TrackedMemoryBudget,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<T, B> PartialEq for TrackedBox<T, B>
where
    T: ConservativeTrackedSize + PartialEq,
    B: TrackedMemoryBudget,
{
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl<T, B> Eq for TrackedBox<T, B>
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

    use crate::{ConservativeTrackedSize, SizeDelta, TrackedBox, TrackedMemoryBudget, TrackedSizeDelta};

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

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Value {
        retained: usize,
        marker: u64,
    }

    impl ConservativeTrackedSize for Value {
        fn conservative_tracked_size(&self) -> usize {
            self.retained
        }
    }

    impl TrackedSizeDelta for Value {
        fn size_delta<R>(&mut self, action: impl FnOnce(&mut Self) -> R) -> (R, SizeDelta) {
            let before = self.retained;
            let result = action(self);
            let delta = match self.retained.cmp(&before) {
                std::cmp::Ordering::Greater => SizeDelta::Increase(self.retained - before),
                std::cmp::Ordering::Less => SizeDelta::Decrease(before - self.retained),
                std::cmp::Ordering::Equal => SizeDelta::Unchanged,
            };
            (result, delta)
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
    fn update_applies_the_values_delta_and_preserves_the_action_result() {
        let budget = RecordingBudget::default();
        let pointer = size_of::<TrackedBox<Value, RecordingBudget>>();
        let fixed_allocation = size_of::<RecordingBudget>();
        let mut value = TrackedBox::new(Value { retained: 40, marker: 1 }, budget.clone());

        assert_eq!(size_of_val(&value), size_of::<usize>());
        assert_eq!(
            value.update(|value| {
                value.retained = 55;
                value.marker = 2;
                "grown"
            }),
            "grown"
        );
        value.update(|_| ());
        value.update(|value| value.retained = 50);
        assert_eq!(value.marker, 2);
        drop(value);

        assert_eq!(
            budget.deltas(),
            vec![
                SizeDelta::Increase(40 + fixed_allocation + pointer),
                SizeDelta::Increase(15),
                SizeDelta::Unchanged,
                SizeDelta::Decrease(5),
                SizeDelta::Decrease(pointer),
                SizeDelta::Decrease(50 + fixed_allocation),
            ]
        );
    }

    #[test]
    fn clone_owns_an_independently_mutable_value() {
        let budget = RecordingBudget::default();
        let original = TrackedBox::new(Value { retained: 40, marker: 1 }, budget.clone());
        let mut clone = original.clone();

        clone.update(|value| {
            value.retained = 50;
            value.marker = 2;
        });

        assert_eq!(original.marker, 1);
        assert_eq!(original.retained, 40);
        assert_eq!(clone.marker, 2);
        assert_eq!(clone.retained, 50);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_tracked_box() {
        let mut criterion = Criterion::default();
        let original = TrackedBox::new(Value { retained: 40, marker: 1 }, BenchmarkBudget::default());
        criterion.bench_function("memory/unit/tracked_box/update", |bencher| {
            bencher.iter_batched_ref(
                || original.clone(),
                |value| {
                    std::hint::black_box(value.update(|value| {
                        value.marker = std::hint::black_box(value.marker + 1);
                    }));
                },
                BatchSize::SmallInput,
            );
        });
        criterion.final_summary();
    }
}
