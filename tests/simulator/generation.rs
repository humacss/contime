use crate::support::{Change, Time};
use contime::simulator::{EventStream, StreamConfig};
use std::num::NonZeroU64;

pub fn events() -> Vec<Change> {
    let mut sequence = 0u64;
    EventStream::new(
        StreamConfig { event_count: 100, seed: 42, settings: NonZeroU64::new(4).unwrap() },
        |_, random| {
            sequence += 1;
            Change {
                id: u128::from(sequence),
                at: Time(sequence),
                targets: Vec::new(),
                delta: random.below(NonZeroU64::new(10).unwrap()) as i64,
            }
        },
        |config, random, event| {
            let first = random.below(config.settings);
            event.targets = vec![u128::from(first + 1), u128::from((first + 1) % config.settings.get() + 1)];
        },
    )
    .collect()
}
