use crate::MemoryChange;

impl MemoryChange {
    pub fn between(before: usize, after: usize) -> Self {
        match after.cmp(&before) {
            std::cmp::Ordering::Greater => Self::Increase(after - before),
            std::cmp::Ordering::Less => Self::Decrease(before - after),
            std::cmp::Ordering::Equal => Self::Unchanged,
        }
    }
}

#[cfg(test)]
mod tests {
    use criterion::Criterion;

    use crate::MemoryChange;

    #[test]
    fn calculates_every_change_direction_without_signed_arithmetic() {
        assert_eq!(MemoryChange::between(1, 2), MemoryChange::Increase(1));
        assert_eq!(MemoryChange::between(2, 1), MemoryChange::Decrease(1));
        assert_eq!(MemoryChange::between(2, 2), MemoryChange::Unchanged);
        assert_eq!(MemoryChange::between(0, 0), MemoryChange::Unchanged);
        assert_eq!(MemoryChange::between(0, usize::MAX), MemoryChange::Increase(usize::MAX));
        assert_eq!(MemoryChange::between(usize::MAX, 0), MemoryChange::Decrease(usize::MAX));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_change() {
        let mut criterion = Criterion::default();
        criterion.bench_function("memory/change/1000", |bencher| {
            bencher.iter(|| {
                for index in 0..1_000 {
                    std::hint::black_box(MemoryChange::between(index, 1_000 - index));
                }
            });
        });
        criterion.final_summary();
    }
}
