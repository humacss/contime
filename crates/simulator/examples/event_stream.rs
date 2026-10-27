//! Generate events without starting ConTime: cargo run -p contime-simulator --example event_stream
use contime_simulator::{EventStream, StreamConfig};
use std::num::NonZeroU64;

struct Settings {
    snapshots: NonZeroU64,
    hot_snapshots: NonZeroU64,
    hot_percent: u64,
}

struct CounterChanged {
    event_id: u64,
    counter_id: u64,
    delta: u64,
}

fn main() {
    // These fixed settings are valid: 0 < hot snapshots < snapshots, share <= 100.
    // Applications accepting user input should validate their settings first.
    let config = StreamConfig {
        event_count: 100_000,
        seed: 42,
        settings: Settings { snapshots: NonZeroU64::new(100).unwrap(), hot_snapshots: NonZeroU64::new(10).unwrap(), hot_percent: 80 },
    };
    let mut sequence = 0;
    let events = EventStream::new(
        config,
        |_, random| {
            sequence += 1;
            CounterChanged {
                event_id: sequence, // Sequential IDs; no content hash to refresh after assignment.
                counter_id: 0,
                delta: 1 + random.below(NonZeroU64::new(5).unwrap()),
            }
        },
        |config, random, event| {
            let settings = &config.settings;
            event.counter_id = if random.below(NonZeroU64::new(100).unwrap()) < settings.hot_percent {
                random.below(settings.hot_snapshots)
            } else {
                let cold = NonZeroU64::new(settings.snapshots.get() - settings.hot_snapshots.get()).unwrap();
                settings.hot_snapshots.get() + random.below(cold)
            };
        },
    );

    let mut counts = [0_u64; 100];
    let mut total = 0;
    let mut last_id = 0;
    for event in events {
        // Replace this local accumulation with submission to your ConTime instance.
        counts[event.counter_id as usize] += 1;
        total += event.delta;
        last_id = event.event_id;
    }
    println!("Generated {last_id} events; total counter increment: {total}");
    println!("Events targeting the 10 hot counters: {}", counts[..10].iter().sum::<u64>());
    println!("Events targeting the 90 other counters: {}", counts[10..].iter().sum::<u64>());
}
