use super::generation;
use contime::{checkpoints::Event, Input};

#[test]
fn independent_factories_produce_repeatable_events_compatible_with_input_contracts() {
    let events = generation::events();
    assert_eq!(events, generation::events());
    assert_eq!(events.len(), 100);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.event_id(), (index + 1) as u128);
        assert_eq!(event.time().0, (index + 1) as u64);
        let mut targets = Vec::new();
        event.snapshot_ids(&mut |id| targets.push(id));
        assert_eq!(targets.len(), 2);
        assert_ne!(targets[0], targets[1]);
        assert!(targets.iter().all(|id| (1..=4).contains(id)));
    }
}
