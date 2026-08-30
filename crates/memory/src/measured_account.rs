use crate::{ConservativeTrackedSize, MeasuredAccount, MemoryAccount, MemoryChange};

impl<T> MemoryAccount<T> for MeasuredAccount
where
    T: ConservativeTrackedSize,
{
    fn new(_value: &T) -> Self {
        Self
    }

    fn current(&self, value: &T) -> usize {
        value.conservative_tracked_size()
    }

    fn change<R, F>(&mut self, value: &mut T, action: F) -> (R, MemoryChange)
    where
        F: FnOnce(&mut T) -> R,
    {
        let before = value.conservative_tracked_size();
        let result = action(value);
        let after = value.conservative_tracked_size();
        (result, MemoryChange::between(before, after))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::mem::size_of;

    use criterion::Criterion;

    use crate::{ConservativeTrackedSize, MeasuredAccount, MemoryAccount, MemoryChange};

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

    #[test]
    fn measures_current_values_and_each_side_of_a_change() {
        let mut value = Value { bytes: 8, measurements: Cell::new(0) };
        let mut account = MeasuredAccount::new(&value);
        assert_eq!(value.measurements.get(), 0);
        assert_eq!(account.current(&value), 8);
        assert_eq!(value.measurements.get(), 1);

        let (result, change) = account.change(&mut value, |value| {
            value.bytes = 13;
            "changed"
        });
        assert_eq!(result, "changed");
        assert_eq!(change, MemoryChange::Increase(5));
        assert_eq!(value.measurements.get(), 3);
        assert_eq!(size_of::<MeasuredAccount>(), 0);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_measured_account() {
        let mut criterion = Criterion::default();
        criterion.bench_function("memory/account/measured/1000", |bencher| {
            let mut value = Value { bytes: 8, measurements: Cell::new(0) };
            let mut account = MeasuredAccount::new(&value);
            bencher.iter(|| {
                for _ in 0..1_000 {
                    std::hint::black_box(account.change(&mut value, |_| ()));
                }
            });
        });
        criterion.final_summary();
    }
}
