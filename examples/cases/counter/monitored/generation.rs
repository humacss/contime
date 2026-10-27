//! Two simulator callbacks: make a counter event, then assign its destinations.
use super::{model::CounterChanged, Options, Time};
use contime_simulator::{EventStream, StreamConfig};

pub fn events(options: Options, start: u64, count: u64, time: Time) -> impl Iterator<Item = CounterChanged> {
    let mut sequence = start;
    EventStream::new(
        StreamConfig { event_count: count, seed: start ^ 42, settings: options },
        move |_, _| {
            sequence += 1;
            CounterChanged { id: sequence.into(), time, targets: Vec::new(), value: 1 }
        },
        |config, random, event| {
            crate::monitored::assign_snapshot_ids(&config.settings, random, &mut event.targets);
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn events_keep_unique_ids_and_distinct_destinations_at_full_fanout() {
        let options = Options { snapshots: 3, targets: 3, snapshot_skew: 100, ..Options::default() };
        let generated: Vec<_> = events(options, 100, 10, Time(7)).collect();
        assert_eq!(generated.len(), 10);
        for (index, event) in generated.iter().enumerate() {
            assert_eq!(event.id, 101 + index as u128);
            assert_eq!(event.targets, [1, 2, 3]);
            assert_eq!(event.time, Time(7));
        }
    }
    #[test]
    fn hotspot_selection_concentrates_traffic_without_forcing_coverage() {
        let options = Options { snapshots: 100, snapshot_skew: 100, ..Options::default() };
        assert!(events(options, 0, 1_000, Time(1)).all(|event| event.targets == [1]));
    }
}
