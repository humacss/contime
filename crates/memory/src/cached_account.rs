use crate::{CachedAccount, ConservativeTrackedSize, MemoryAccount, MemoryChange};

impl<T> MemoryAccount<T> for CachedAccount
where
    T: ConservativeTrackedSize,
{
    fn new(value: &T) -> Self {
        Self { bytes: value.conservative_tracked_size() }
    }

    fn current(&self, _value: &T) -> usize {
        self.bytes
    }

    fn change<R, F>(&mut self, value: &mut T, action: F) -> (R, MemoryChange)
    where
        F: FnOnce(&mut T) -> R,
    {
        let before = self.bytes;
        let result = action(value);
        let after = value.conservative_tracked_size();
        self.bytes = after;
        (result, MemoryChange::between(before, after))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::mem::size_of;

    use criterion::Criterion;

    use crate::{CachedAccount, ConservativeTrackedSize, MemoryAccount, MemoryChange};

    struct Value {
        bytes: usize,
        measurements: Cell<usize>,
    }

    impl ConservativeTrackedSize for Value {
        fn conservative_tracked_size(&self) -> usize {
            self.measurements.set(self.measurements.get() + 1);
            self.bytes
        }
    }

    struct Expensive(Vec<usize>);

    impl ConservativeTrackedSize for Expensive {
        fn conservative_tracked_size(&self) -> usize {
            self.0.iter().copied().fold(0, usize::saturating_add)
        }
    }

    #[test]
    fn caches_current_size_and_measures_once_after_each_change() {
        let mut value = Value { bytes: 8, measurements: Cell::new(0) };
        let mut account = CachedAccount::new(&value);
        assert_eq!(value.measurements.get(), 1);
        assert_eq!(account.current(&value), 8);
        assert_eq!(value.measurements.get(), 1);

        let (result, change) = account.change(&mut value, |value| {
            value.bytes = 3;
            "changed"
        });
        assert_eq!(result, "changed");
        assert_eq!(change, MemoryChange::Decrease(5));
        assert_eq!(account.current(&value), 3);
        assert_eq!(value.measurements.get(), 2);
        assert_eq!(size_of::<CachedAccount>(), size_of::<usize>());
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_cached_account() {
        let mut criterion = Criterion::default();
        criterion.bench_function("memory/account/cached/1000", |bencher| {
            let mut value = Value { bytes: 8, measurements: Cell::new(0) };
            let mut account = CachedAccount::new(&value);
            bencher.iter(|| {
                for _ in 0..1_000 {
                    std::hint::black_box(account.change(&mut value, |_| ()));
                }
            });
        });
        criterion.bench_function("memory/account/cached_expensive/1000", |bencher| {
            let mut value = Expensive(vec![1; 1_000]);
            let mut account = CachedAccount::new(&value);
            bencher.iter(|| {
                for _ in 0..1_000 {
                    std::hint::black_box(account.change(&mut value, |_| ()));
                }
            });
        });
        criterion.final_summary();
    }
}
