//! Composable application definitions and support used by the derives.
//! Modules register event and snapshot lanes with one shared time type.
pub use crate::contime_core::module::*;

/// Hashes typed local identity fields without binary serialization.
///
/// IDs use the full first 128 bits of BLAKE3 through Content ID's local hashing
/// adapter. Rust `Hash` and type names do not define a stable wire format: keep
/// explicit protocol IDs when identities must survive compiler/type changes.
#[cfg(feature = "macros")]
#[doc(hidden)]
pub fn identity<Owner: ?Sized, Value: std::hash::Hash + ?Sized>(value: &Value) -> u128 {
    struct IdentityHasher(blake3::Hasher);
    impl std::hash::Hasher for IdentityHasher {
        fn finish(&self) -> u64 {
            let digest = self.0.finalize();
            let mut bytes = [0; 8];
            bytes.copy_from_slice(&digest.as_bytes()[..8]);
            u64::from_le_bytes(bytes)
        }
        fn write(&mut self, bytes: &[u8]) {
            self.0.update(bytes);
        }
    }
    let mut hasher = IdentityHasher(blake3::Hasher::new());
    {
        let mut content = contentid::LocalContentHasher::with_hasher(&mut hasher);
        content.update_value(std::any::type_name::<Owner>());
        content.update_value(value);
    }
    let digest = hasher.0.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    u128::from_le_bytes(bytes)
}

#[cfg(all(test, feature = "macros"))]
mod tests {
    use super::identity;
    struct Counter;
    struct Statistics;

    #[test]
    fn identities_are_repeatable_and_separate_types_and_values() {
        let first = identity::<Counter, _>(&(7_u64, 10_i64));
        assert_eq!(first, identity::<Counter, _>(&(7_u64, 10_i64)));
        assert_ne!(first, identity::<Counter, _>(&(7_u64, 11_i64)));
        assert_ne!(first, identity::<Statistics, _>(&(7_u64, 10_i64)));
    }
}

#[cfg(all(test, feature = "macros"))]
mod benchmarks {
    use super::identity;
    use criterion::{black_box, Criterion};
    struct Counter;

    #[test]
    #[ignore = "optimized local identity benchmark"]
    fn typed_local_identity() {
        let payload = (7_u64, -500_i64, 3_i64);
        assert_eq!(identity::<Counter, _>(&payload), identity::<Counter, _>(&payload));
        let mut criterion = Criterion::default();
        criterion.bench_function("module/typed_local_identity", |b| {
            b.iter(|| black_box(identity::<Counter, _>(black_box(&payload))));
        });
        criterion.final_summary();
    }
}
