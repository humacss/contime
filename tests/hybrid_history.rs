use std::collections::{BTreeMap, HashSet};

use contime::{HistoryInputs, Input, TestEvent};

fn next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *state
}

fn actual_keys(history: &HistoryInputs<i64, TestEvent>) -> Vec<(i64, u128)> {
    history.entries().map(|(time, id, _input)| (time, id)).collect()
}

#[test]
fn hybrid_history_matches_a_canonical_btree_across_ordered_late_and_pruned_inputs() {
    for seed in 0_u64..32 {
        let mut random = seed.wrapping_add(1);
        let mut history = HistoryInputs::<i64, TestEvent>::new();
        let mut reference = BTreeMap::<(i64, u128), TestEvent>::new();
        let mut retained_ids = HashSet::<u128>::new();
        let mut next_id = 1_u128;
        let mut horizon = 0_i64;
        let mut latest_time = 0_i64;

        for operation in 0..1_000 {
            let roll = next_u64(&mut random) % 100;
            if roll < 95 {
                let duplicate_id = operation % 23 == 0 && !retained_ids.is_empty();
                let id = if duplicate_id {
                    *retained_ids.iter().min().expect("the duplicate branch requires one retained ID")
                } else {
                    let id = next_id;
                    next_id += 1;
                    id
                };
                let time = if roll < 80 || latest_time == horizon {
                    latest_time + i64::try_from(next_u64(&mut random) % 3).expect("small offset fits i64")
                } else {
                    let retained_width = u64::try_from(latest_time - horizon).expect("the horizon never exceeds latest time");
                    horizon + i64::try_from(next_u64(&mut random) % (retained_width + 1)).expect("retained offset fits i64")
                };
                latest_time = latest_time.max(time);

                if retained_ids.insert(id) {
                    let input = TestEvent::Positive(1, time, id, 1);
                    reference.insert((time, id), input.clone());
                    history.insert_batch(vec![input]);
                }
            } else {
                let remaining = u64::try_from(latest_time - horizon).expect("the horizon never exceeds latest time");
                horizon += i64::try_from(next_u64(&mut random) % (remaining + 1)).expect("horizon step fits i64");

                let removed_keys = reference.range(..(horizon, u128::MIN)).map(|(key, _input)| *key).collect::<Vec<_>>();
                let expected_bytes = removed_keys
                    .iter()
                    .map(|key| reference.get(key).expect("a collected reference key must exist").conservative_size())
                    .sum::<u64>();
                for key in &removed_keys {
                    let removed = reference.remove(key).expect("a collected reference key must be removable");
                    retained_ids.remove(&removed.id());
                }

                let (actual_count, actual_bytes) = history.prune_before_time(horizon);
                assert_eq!(actual_count, removed_keys.len(), "seed {seed}, operation {operation}: prune count diverged");
                assert_eq!(actual_bytes, expected_bytes, "seed {seed}, operation {operation}: prune bytes diverged");
            }

            let expected_keys = reference.keys().copied().collect::<Vec<_>>();
            assert_eq!(actual_keys(&history), expected_keys, "seed {seed}, operation {operation}: canonical order diverged");
            assert_eq!(history.len(), reference.len(), "seed {seed}, operation {operation}: length diverged");
            assert_eq!(history.is_empty(), reference.is_empty(), "seed {seed}, operation {operation}: emptiness diverged");
            assert_eq!(
                history.latest_entry_key(),
                reference.keys().next_back().copied(),
                "seed {seed}, operation {operation}: tip diverged"
            );
        }
    }
}
