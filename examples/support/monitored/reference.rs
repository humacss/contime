//! Canonical example references and validation, shared by blocking and frame hosts.
use super::Failure;
use crate::definition::Example;
use contime::{
    checkpoints::{ApplyEvents, Event, Snapshot},
    Input,
};
pub fn record<M: Example>(events: &mut [M::Event], expected: &mut [M::State], maximum: usize, ids: &mut Vec<u128>) -> Result<u64, Failure> {
    events.sort_by_key(|event| (event.time(), event.event_id()));
    let mut applications = 0;
    for event in events {
        ids.clear();
        event.snapshot_ids(&mut |id| ids.push(id));
        ids.sort_unstable();
        if ids.is_empty() || ids.len() > maximum || ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("Simulator produced empty, duplicate, or excessive snapshot destinations".into());
        }
        for &id in ids.iter() {
            let index = id
                .checked_sub(1)
                .and_then(|id| usize::try_from(id).ok())
                .filter(|&index| index < expected.len())
                .ok_or("Simulator produced an unknown snapshot ID")?;
            let state = &mut expected[index];
            if M::count(state) == 0 {
                *state = M::State::create(id, event);
            }
            M::record(state, event);
            state.set_time(event.time());
            applications += 1;
        }
    }
    Ok(applications)
}
pub fn receive<M: Example>(states: Vec<Box<M::State>>, actual: &mut [M::State], seen: &mut [bool]) -> Result<(), Failure> {
    for state in states {
        let index = M::id(&state)
            .checked_sub(1)
            .and_then(|id| usize::try_from(id).ok())
            .filter(|&index| index < actual.len())
            .ok_or("Unknown queried snapshot")?;
        if seen[index] {
            return Err("Duplicate queried snapshot".into());
        }
        seen[index] = true;
        actual[index] = *state;
    }
    Ok(())
}
pub fn verify<M: Example>(actual: &[M::State], expected: &[M::State]) -> Result<(), Failure> {
    if actual.len() != expected.len() {
        return Err("Snapshot validation length mismatch".into());
    }
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        if M::count(actual) != M::count(expected) || M::value(actual) != M::value(expected) || M::hash(actual) != M::hash(expected) {
            return Err(format!("Snapshot {} differs from generated reference (count, value, or full digest)", index + 1).into());
        }
    }
    Ok(())
}
