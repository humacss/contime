use super::{AdvanceTime, Timestamp};

macro_rules! integers {
    ($valid:expr; $($ty:ty),* $(,)?) => { $(
        impl Timestamp for $ty {
            fn minimum() -> Self { Self::MIN }
            fn previous(&self) -> Option<Self> { self.checked_sub(1) }
        }
        impl AdvanceTime for $ty {
            fn saturating_sub(&self, retention: &Self) -> Self { <$ty>::saturating_sub(*self, *retention) }
            fn is_valid_retention(&self) -> bool { ($valid)(*self) }
        }
    )* };
}
integers!(|_| true; u8, u16, u32, u64, u128, usize);
integers!(|time| time >= 0; i8, i16, i32, i64, i128, isize);

#[cfg(test)]
mod tests {
    use super::{AdvanceTime, Timestamp};

    #[test]
    fn integer_domains_preserve_their_actual_bounds() {
        macro_rules! check {
            ($($ty:ty),*) => {$({
                assert_eq!(<$ty as Timestamp>::minimum(), <$ty>::MIN);
                assert_eq!(Timestamp::previous(&<$ty>::MIN), None);
                assert_eq!(Timestamp::previous(&(<$ty>::MIN + 1)), Some(<$ty>::MIN));
                assert_eq!(AdvanceTime::saturating_sub(&<$ty>::MIN, &1), <$ty>::MIN);
                assert!(AdvanceTime::is_valid_retention(&(0 as $ty)));
                assert!(AdvanceTime::is_valid_retention(&<$ty>::MAX));
            })*};
        }
        check!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);
    }

    #[test]
    fn signed_upper_bound_and_zero_retention_preserve_domain_values() {
        assert_eq!(Timestamp::previous(&i64::MAX), Some(i64::MAX - 1));
        for time in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert_eq!(AdvanceTime::saturating_sub(&time, &0), time);
        }
        assert_eq!(AdvanceTime::saturating_sub(&i64::MAX, &i64::MAX), 0);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_signed_time_arithmetic() {
        use criterion::{black_box, Criterion, Throughput};

        // Prepare a mixed domain workload before Criterion starts timing.
        let times = [i64::MIN, i64::MIN + 1, -1000, -1, 0, 1, 1000, i64::MAX];
        let retention = 100i64;
        let mut criterion = Criterion::default();
        let mut group = criterion.benchmark_group("time/signed_i64");
        group.throughput(Throughput::Elements(times.len() as u64));
        group.bench_function("previous_and_retention", |b| {
            b.iter(|| {
                for time in black_box(&times) {
                    black_box(Timestamp::previous(black_box(time)));
                    black_box(AdvanceTime::saturating_sub(black_box(time), black_box(&retention)));
                }
            });
        });
        group.finish();
        criterion.final_summary();
    }

    #[test]
    fn signed_retention_crosses_zero_and_saturates_at_minimum() {
        assert_eq!(AdvanceTime::saturating_sub(&2i64, &5), -3);
        assert_eq!(AdvanceTime::saturating_sub(&-3i64, &5), -8);
        assert_eq!(AdvanceTime::saturating_sub(&(i64::MIN + 1), &5), i64::MIN);
        assert_eq!(Timestamp::previous(&0i64), Some(-1));
        assert!(!AdvanceTime::is_valid_retention(&-1i64));
        assert!(!AdvanceTime::is_valid_retention(&i64::MIN));
    }
}
