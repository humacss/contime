//! Model regression preserved from the retired ordered example runner.
use super::*;
#[test]
fn chained_hash_detects_reordering_and_continues_from_a_snapshot() {
    let first = CounterChanged { event_id: 1, counter_ids: vec![1, 2], time: Time(1), delta: 3 };
    let second = CounterChanged { event_id: 2, counter_ids: vec![1], time: Time(2), delta: 4 };
    let initial = [0; 32];
    let checkpoint = chain_hash(&initial, &first);
    let ordered = chain_hash(&checkpoint, &second);
    let reversed = chain_hash(&chain_hash(&initial, &second), &first);
    assert_ne!(ordered, reversed);
    let mut snapshot = Counter::default();
    snapshot.apply_change(&first);
    let mut restored = snapshot.clone();
    restored.apply_change(&second);
    assert_eq!(restored.hash, ordered);
    assert_eq!(restored.event_count, 2);
    assert_eq!(restored.value, 7);
    let changed = CounterChanged { delta: 5, ..second };
    assert_ne!(ordered, chain_hash(&checkpoint, &changed));
}
