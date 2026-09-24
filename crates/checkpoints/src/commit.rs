//! Restricted checkpoint and boundary updates for caller-defined commit policies.
use crate::{Checkpoint, Snapshot, Store};

pub struct Commit<'a, S: Snapshot, H> {
    store: &'a mut Store<S, H>,
    checkpoint: &'a Checkpoint<S>,
    checkpoint_index: &'a mut usize,
}

impl<S: Snapshot, H> Commit<'_, S, H> {
    pub fn checkpoint_index(&self) -> usize {
        *self.checkpoint_index
    }

    /// Replaces an existing slot with the working checkpoint.
    /// The policy must preserve timestamp ordering and checkpoint validity.
    pub fn replace_checkpoint(&mut self, index: usize) {
        self.store.checkpoints[index] = self.checkpoint.clone();
        *self.checkpoint_index = index;
    }

    /// Appends the working checkpoint and continues from its new index.
    pub fn push_checkpoint(&mut self) {
        self.store.checkpoints.push_back(self.checkpoint.clone());
        *self.checkpoint_index = self.store.checkpoints.len() - 1;
    }

    /// Moves forward through existing slots, then replaces the current slot.
    /// Older slots remain available for later physical cleanup.
    pub fn forward_checkpoint(&mut self) {
        while self
            .store
            .checkpoints
            .get(*self.checkpoint_index + 1)
            .is_some_and(|next| next.snapshot.time() <= self.checkpoint.snapshot.time())
        {
            *self.checkpoint_index += 1;
        }
        self.replace_checkpoint(self.checkpoint_index());
    }

    /// Records the inclusive valid-through boundary after contiguous replay.
    pub fn set_dirty(&mut self, time: S::Time) {
        self.store.dirty = time;
    }

    /// Publishes a completed forwarding boundary. The policy must first retain
    /// a valid checkpoint immediately before it and must not move it backwards.
    pub fn set_horizon(&mut self, time: S::Time) {
        self.store.horizon = time;
    }
}

pub(crate) fn commit<S: Snapshot, H, R>(
    store: &mut Store<S, H>,
    checkpoint: &Checkpoint<S>,
    checkpoint_index: &mut usize,
    policy: impl FnOnce(&mut Commit<'_, S, H>) -> R,
) -> R {
    policy(&mut Commit { store, checkpoint, checkpoint_index })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::testing::{TestSnapshot, Time};
    use std::hint::black_box;

    #[rstest::rstest]
    #[case::replace(false, 0, 1)]
    #[case::push(true, 1, 2)]
    fn happy(#[case] push: bool, #[case] expected_index: usize, #[case] expected_len: usize) {
        let expected_time: Time = 10;
        let expected_dirty = expected_time;
        let expected_horizon: Time = 5;
        let expected_events = [1, 2];

        let mut store = Store::new(expected_events, TestSnapshot { time: 0, sum: 0 }, 2);
        let working = Checkpoint { snapshot: TestSnapshot { time: expected_time, sum: 0 }, history_event_count: 1 };
        let mut index = 0;

        let actual_index = commit(&mut store, &working, &mut index, |access| {
            if push {
                access.push_checkpoint();
            } else {
                access.replace_checkpoint(0);
            }
            access.set_dirty(expected_dirty);
            access.set_horizon(expected_horizon);
            access.checkpoint_index()
        });
        let actual_time = *store.checkpoints[index].snapshot.time();
        let actual_len = store.checkpoints.len();

        assert_eq!(actual_index, expected_index);
        assert_eq!(index, expected_index);
        assert_eq!(actual_len, expected_len);
        assert_eq!(actual_time, expected_time);
        assert_eq!(store.dirty, expected_dirty);
        assert_eq!(store.horizon, expected_horizon);
        assert_eq!(store.events, expected_events);
    }

    #[test]
    fn no_op_preserves_storage_and_index() {
        let expected_time: Time = 0;
        let expected_index = 0;
        let expected_len = 1;

        let mut store = Store::new((), TestSnapshot { time: expected_time, sum: 0 }, 2);
        let working = Checkpoint { snapshot: TestSnapshot { time: 10, sum: 0 }, history_event_count: 1 };
        let mut index = expected_index;

        commit(&mut store, &working, &mut index, |_| {});
        let actual_time = *store.checkpoints[index].snapshot.time();

        assert_eq!(actual_time, expected_time);
        assert_eq!(index, expected_index);
        assert_eq!(store.checkpoints.len(), expected_len);
        assert_eq!(store.dirty, expected_time);
        assert_eq!(store.horizon, expected_time);
    }

    #[test]
    fn replacement_preserves_other_slots() {
        let expected_times = [0, 15, 20];
        let expected_index = 1;
        let expected_count = 3;

        let mut store = Store::new((), TestSnapshot { time: 0, sum: 0 }, 2);
        store
            .checkpoints
            .extend([10, 20].map(|time| Checkpoint { snapshot: TestSnapshot { time, sum: 0 }, history_event_count: time / 10 }));
        let working = Checkpoint { snapshot: TestSnapshot { time: 15, sum: 0 }, history_event_count: expected_count };
        let mut index = 0;

        commit(&mut store, &working, &mut index, |access| access.replace_checkpoint(expected_index));
        let actual_times = store.checkpoints.iter().map(|checkpoint| *checkpoint.snapshot.time()).collect::<Vec<_>>();
        let actual_count = store.checkpoints[index].history_event_count;

        assert_eq!(actual_times, expected_times);
        assert_eq!(index, expected_index);
        assert_eq!(actual_count, expected_count);
    }

    #[rstest::rstest]
    #[case::same_slot(5, 0, &[5, 10, 20, 30])]
    #[case::next_slot(15, 1, &[0, 15, 20, 30])]
    #[case::several_slots(25, 2, &[0, 10, 25, 30])]
    #[case::past_tip(40, 3, &[0, 10, 20, 40])]
    fn forwarding_reuses_slots_from_the_current_index(
        #[case] target: Time,
        #[case] expected_index: usize,
        #[case] expected_times: &[Time],
    ) {
        let expected_dirty = 0;
        let expected_horizon = 0;

        let mut store = Store::new((), TestSnapshot { time: 0, sum: 0 }, 100);
        store
            .checkpoints
            .extend([10, 20, 30].map(|time| Checkpoint { snapshot: TestSnapshot { time, sum: 0 }, history_event_count: time }));
        let working = Checkpoint { snapshot: TestSnapshot { time: target, sum: 7 }, history_event_count: 3 };
        let mut index = 0;

        commit(&mut store, &working, &mut index, |access| access.forward_checkpoint());
        let actual_times = store.checkpoints.iter().map(|checkpoint| checkpoint.snapshot.time).collect::<Vec<_>>();

        assert_eq!(actual_times, expected_times);
        assert_eq!(index, expected_index);
        assert_eq!(store.dirty, expected_dirty);
        assert_eq!(store.horizon, expected_horizon);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_commit_unit() {
        let mut store = Store::new((), TestSnapshot { time: 0, sum: 0 }, 100);
        let working = Checkpoint { snapshot: TestSnapshot { time: 10, sum: 0 }, history_event_count: 100 };
        let mut index = 0;
        let mut criterion = criterion::Criterion::default()
            .warm_up_time(std::time::Duration::from_millis(200))
            .measurement_time(std::time::Duration::from_secs(1))
            .sample_size(30);
        criterion.bench_function("checkpoints/unit/commit/replace_and_set_dirty", |b| {
            b.iter(|| {
                commit(black_box(&mut store), black_box(&working), black_box(&mut index), |access| {
                    access.replace_checkpoint(black_box(0));
                    access.set_dirty(black_box(10));
                });
                black_box((&store.checkpoints, store.dirty, index));
            });
        });
        criterion.final_summary();
    }
}
