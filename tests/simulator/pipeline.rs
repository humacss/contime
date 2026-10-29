use super::generation;
use crate::support::{self, Change, Counter, Time};
use std::collections::BTreeMap;

fn verify(states: impl IntoIterator<Item = Box<Counter>>, events: &[Change]) {
    let mut expected = BTreeMap::<u128, (i64, Vec<u128>)>::new();
    let mut canonical: Vec<_> = events.iter().collect();
    canonical.sort_by_key(|e| (e.at, e.id));
    for event in canonical {
        for &id in &event.targets {
            let state = expected.entry(id).or_default();
            state.0 += event.delta;
            state.1.push(event.id);
        }
    }
    let actual: BTreeMap<_, _> = states.into_iter().map(|s| (s.id, (s.value, s.order.clone()))).collect();
    assert_eq!(actual, expected);
}

#[cfg(feature = "runtime")]
#[test]
fn runtime_replays_late_generated_events_in_canonical_order() {
    for workers in [1, 2] {
        let events = generation::events();
        let engine = support::runtime(workers);
        engine.apply(events.iter().step_by(2).cloned()).unwrap();
        engine.advance_to(Time(100)).unwrap();
        engine.wait_until_idle(std::time::Duration::from_secs(5)).unwrap();
        engine.apply(events.iter().skip(1).step_by(2).rev().cloned()).unwrap();
        engine.wait_until_idle(std::time::Duration::from_secs(5)).unwrap();
        verify(engine.query_at(Time(100), 1..=4).unwrap(), &events);
        assert!(engine.errors().is_empty());
        engine.shutdown();
    }
}

#[cfg(feature = "manual")]
#[test]
fn manual_replays_the_same_generated_events_without_background_processing() {
    use std::{cell::RefCell, rc::Rc};
    let events = generation::events();
    let mut engine = support::manual();
    engine.apply(events.iter().step_by(2).cloned()).unwrap();
    engine.advance_to(Time(100)).unwrap();
    support::drive(&mut engine);
    engine.apply(events.iter().skip(1).step_by(2).rev().cloned()).unwrap();
    support::drive(&mut engine);
    let received = Rc::new(RefCell::new(None));
    let output = received.clone();
    engine.query_at(Time(100), 1..=4, move |states| *output.borrow_mut() = Some(states)).unwrap();
    support::drive(&mut engine);
    verify(received.borrow_mut().take().unwrap(), &events);
    assert!(engine.take_rejections().is_empty());
}
