//! Simulator callbacks create canonical events; shared delivery reverses each group.
use super::model::CounterChanged;
use crate::{
    definition::Time,
    monitored::{self, Options},
};
use contime_simulator::{EventStream, Random, StreamConfig};
pub fn events(options: Options, start: u64, count: u64, time: Time) -> impl Iterator<Item = CounterChanged> {
    let mut selection_random = Random::new(start ^ 42);
    let mut primary = Vec::new();
    // Choose one shared destination so reversing the group exercises one history.
    let mut primary_options = options.clone();
    primary_options.targets = 1;
    primary_options.average_scaled = None;
    monitored::assign_snapshot_ids(&primary_options, &mut selection_random, &mut primary);
    let primary = primary[0];
    let mut sequence = start;
    let first_time = time.0.checked_sub(count).expect("host reserves one timestamp per generated event");
    EventStream::new(
        StreamConfig { event_count: count, seed: start ^ 42, settings: options },
        move |_, _| {
            sequence += 1;
            CounterChanged { event_id: sequence.into(), counter_ids: Vec::new(), time: Time(first_time + sequence - start), delta: 1 }
        },
        move |config, random, event| monitored::assign_with_primary(&config.settings, random, &mut event.counter_ids, Some(primary)),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_hash_events_have_a_common_target_and_strict_canonical_order() {
        let options = Options { snapshots: 8, targets: 3, ..Options::default() };
        let events: Vec<_> = events(options, 10, 16, Time(100)).collect();
        assert!(events.windows(2).all(|pair| pair[0].time < pair[1].time && pair[0].event_id < pair[1].event_id));
        assert!((1..=8).any(|id| events.iter().all(|event| event.counter_ids.contains(&id))));
        assert!(events.iter().all(|event| event.counter_ids.len() == 3));
        assert_eq!(events.last().unwrap().time, Time(100));
    }
}
