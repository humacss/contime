//! Canonical-prefix and invalid-state regressions preserved from the retired oracle.
use crate::definition::Time;
use crate::monitored::reference::*;
use crate::{
    model::{chain_hash, Counter, CounterChanged},
    setup::HashExample,
};
fn event(id: u128, time: u64, ids: &[u128], delta: i64) -> CounterChanged {
    CounterChanged { event_id: id, counter_ids: ids.to_vec(), time: Time(time), delta }
}
fn expected(events: &mut [CounterChanged]) -> Vec<Counter> {
    let mut states = vec![Counter::default(); 3];
    record::<HashExample>(events, &mut states, 3, &mut Vec::new()).unwrap();
    states
}
#[allow(clippy::vec_box)] // Exercise the existing boxed query response transport.
fn check(states: Vec<Box<Counter>>, expected: &[Counter]) -> Result<(), Box<dyn std::error::Error>> {
    let mut actual = vec![Counter::default(); expected.len()];
    receive::<HashExample>(states, &mut actual, &mut vec![false; expected.len()])?;
    verify::<HashExample>(&actual, expected)
}
#[test]
fn canonical_order_uses_timestamp_then_id_and_counts_shared_events_per_target() {
    let first = event(1, 1, &[1, 2], 2);
    let second = event(2, 2, &[1], 3);
    let third = event(3, 2, &[1, 2], 5);
    let mut events = vec![third.clone(), first.clone(), second.clone()];
    let mut states = vec![Counter::default(); 3];
    let count = record::<HashExample>(&mut events, &mut states, 2, &mut Vec::new()).unwrap();
    assert_eq!(count, 5);
    assert_eq!(events.iter().map(|e| e.event_id).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!((states[0].event_count, states[0].value), (3, 10));
    assert_eq!((states[1].event_count, states[1].value), (2, 7));
    assert_eq!(states[0].hash, chain_hash(&chain_hash(&chain_hash(&[0; 32], &first), &second), &third));
    assert_eq!(states[1].hash, chain_hash(&chain_hash(&[0; 32], &first), &third));
}
#[test]
fn captured_prefix_remains_valid_after_later_reference_events() {
    let mut states = expected(&mut [event(1, 1, &[1, 2], 3)]);
    let prefix = states.clone();
    record::<HashExample>(&mut [event(2, 2, &[1], 7)], &mut states, 2, &mut Vec::new()).unwrap();
    let queried = vec![Box::new(prefix[1].clone()), Box::new(prefix[0].clone())];
    check(queried.clone(), &prefix).unwrap();
    assert!(check(queried, &states).is_err());
}
#[test]
fn validation_rejects_count_value_digest_duplicates_unknown_and_missing_states() {
    let wanted = expected(&mut [event(1, 1, &[1, 2], 3), event(2, 2, &[1, 2], 4)]);
    let valid = || vec![Box::new(wanted[0].clone()), Box::new(wanted[1].clone())];
    check(valid(), &wanted).unwrap(); // Untouched snapshot 3 need not be returned.
    for corrupt in 0..4 {
        let mut states = valid();
        match corrupt {
            0 => states[0].event_count += 1,
            1 => states[0].value += 1,
            2 => states[0].hash[31] ^= 1,
            _ => {
                states[0].hash = chain_hash(&chain_hash(&[0; 32], &event(2, 2, &[1, 2], 4)), &event(1, 1, &[1, 2], 3));
            }
        }
        assert!(check(states, &wanted).is_err());
        check(valid(), &wanted).unwrap(); // Failure must not damage the reference.
    }
    assert!(check(vec![Box::new(wanted[0].clone())], &wanted).is_err());
    let mut duplicate = valid();
    duplicate.push(duplicate[0].clone());
    assert!(check(duplicate, &wanted).is_err());
    for id in [0, 4] {
        let mut unknown = valid();
        unknown[0].counter_id = id;
        assert!(check(unknown, &wanted).is_err());
    }
}
