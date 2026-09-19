/// Stable snapshot placement shared by applies, queries, and listeners.
/// By default the full snapshot ID is reduced modulo the worker count.
#[derive(Clone, Copy, Debug, Default)]
pub struct Placement {
    mapper: Option<fn(u128) -> u128>,
}

impl Placement {
    /// Installs a deterministic ID mapping, for example a consumer-owned hash.
    /// Its result must remain stable throughout the lifetime of the topology.
    pub const fn with_mapper(mapper: fn(u128) -> u128) -> Self {
        Self { mapper: Some(mapper) }
    }

    /// Returns an index in a nonempty topology.
    pub fn worker_index(self, snapshot_id: u128, worker_count: usize) -> usize {
        assert!(worker_count > 0, "placement requires at least one worker");
        let value = self.mapper.map_or(snapshot_id, |mapper| mapper(snapshot_id));
        (value % worker_count as u128) as usize
    }
}

#[cfg(test)]
mod tests {
    use crate::Placement;

    #[test]
    fn default_placement_uses_the_full_snapshot_id_without_hashing() {
        let placement = Placement::default();
        for id in [0, 1, 2, 100, 1_u128 << 100, u128::MAX] {
            for workers in [1, 3, 7, 16] {
                assert_eq!(placement.worker_index(id, workers), (id % workers as u128) as usize);
            }
        }
    }

    #[test]
    fn consumers_can_map_ids_before_modulus() {
        let placement = Placement::with_mapper(|id| id.rotate_right(64));
        assert_eq!(placement.worker_index(7_u128 << 64, 3), 1);
        assert_eq!(Placement::default().worker_index(7_u128 << 64, 3), 1);
        assert_eq!(placement.worker_index(1_u128 << 64, 7), 1);
        assert_eq!(Placement::default().worker_index(1_u128 << 64, 7), 2);
    }
}
