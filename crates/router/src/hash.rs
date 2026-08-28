pub(crate) struct RouterHasher {
    state: ahash::RandomState,
}

impl RouterHasher {
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: ahash::RandomState::with_seeds(seed, seed.rotate_left(17), seed.rotate_left(33), seed.rotate_left(49)) }
    }

    pub(crate) fn worker_index(&self, snapshot_id: u128, worker_count: usize) -> usize {
        if worker_count == 1 {
            0
        } else {
            self.state.hash_one(snapshot_id) as usize % worker_count
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::Criterion;

    use super::RouterHasher;

    #[test]
    fn one_worker_always_uses_index_zero() {
        let hasher = RouterHasher::new(7);

        assert_eq!(hasher.worker_index(0, 1), 0);
        assert_eq!(hasher.worker_index(u128::MAX, 1), 0);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_hash() {
        let snapshot_ids = (0..1_000_u128).collect::<Vec<_>>();
        let mut criterion = Criterion::default();

        criterion.bench_function("hash/1000_snapshot_ids/8_workers", |bencher| {
            bencher.iter(|| {
                let hasher = RouterHasher::new(black_box(7));
                let snapshot_ids = black_box(snapshot_ids.as_slice());
                let checksum =
                    snapshot_ids.iter().fold(0_usize, |checksum, snapshot_id| checksum.wrapping_add(hasher.worker_index(*snapshot_id, 8)));
                black_box(checksum)
            });
        });

        criterion.final_summary();
    }
}
