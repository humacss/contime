use contime_simulator::{EventStream, Random, StreamConfig};
use std::{cell::Cell, num::NonZeroU64};

#[derive(Debug, PartialEq, Eq)]
struct Event {
    payload: u64,
    snapshot: String,
}

#[test]
fn creation_is_lazy_and_assignment_happens_before_yielding() {
    let created = Cell::new(0);
    let assigned = Cell::new(0);
    let config = StreamConfig { event_count: 2, seed: 42, settings: "sensor" };
    let mut stream = EventStream::new(
        config,
        |config, _| {
            created.set(created.get() + 1);
            Event { payload: 7, snapshot: config.settings.into() }
        },
        |config, _, event| {
            assert_eq!(event.snapshot, config.settings);
            assigned.set(assigned.get() + 1);
            event.snapshot.push_str("-42");
        },
    );
    assert_eq!((created.get(), assigned.get()), (0, 0));
    assert_eq!(stream.next(), Some(Event { payload: 7, snapshot: "sensor-42".into() }));
    assert_eq!((created.get(), assigned.get()), (1, 1));
    assert_eq!(stream.count(), 1);
    assert_eq!((created.get(), assigned.get()), (2, 2));
}

#[test]
fn empty_and_exhausted_streams_never_invoke_callbacks() {
    let mut stream = EventStream::new(
        StreamConfig { event_count: 0, seed: 0, settings: () },
        |_, _| panic!("empty factory"),
        |_, _, _: &mut ()| panic!("empty assignment"),
    );
    assert_eq!(stream.size_hint(), (0, Some(0)));
    assert_eq!(stream.next(), None);
    assert_eq!(stream.next(), None);
}

fn samples(seed: u64, extra_payload_randomness: bool) -> Vec<(u64, u64)> {
    EventStream::new(
        StreamConfig { event_count: 100, seed, settings: NonZeroU64::new(1_000).unwrap() },
        move |_, random| {
            let payload = random.next_u64();
            if extra_payload_randomness {
                random.next_u64();
            }
            (payload, 0)
        },
        |config, random, event| event.1 = random.below(config.settings),
    )
    .collect()
}

#[test]
fn seed_reproduces_events_and_payload_draws_do_not_change_snapshot_distribution() {
    assert_eq!(samples(42, false), samples(42, false));
    assert_ne!(samples(42, false), samples(43, false));
    let snapshots = |extra| samples(42, extra).into_iter().map(|event| event.1).collect::<Vec<_>>();
    assert_eq!(snapshots(false), snapshots(true));
    assert!(snapshots(false).iter().all(|id| *id < 1_000));
}

#[test]
fn callbacks_can_capture_state_and_assign_multiple_composite_keys() {
    let mut sequence = 0;
    let events: Vec<_> = EventStream::new(
        StreamConfig { event_count: 3, seed: 1, settings: ("site", 2) },
        |_, _| {
            sequence += 1;
            (sequence, Vec::new())
        },
        |config, _, event| {
            for sensor in 0..config.settings.1 {
                event.1.push((config.settings.0, sensor));
            }
        },
    )
    .collect();
    assert_eq!(events, vec![(1, vec![("site", 0), ("site", 1)]), (2, vec![("site", 0), ("site", 1)]), (3, vec![("site", 0), ("site", 1)])]);
}

#[test]
fn iterator_reports_remaining_work_without_truncating_large_counts() {
    let mut stream = EventStream::new(StreamConfig { event_count: u64::MAX, seed: 0, settings: () }, |_, _| 1, |_, _, _| {});
    let (lower, upper) = stream.size_hint();
    assert_eq!(lower, usize::MAX);
    assert_eq!(upper, usize::try_from(u64::MAX).ok());
    assert_eq!(stream.next(), Some(1));
    assert_eq!(stream.size_hint().1, usize::try_from(u64::MAX - 1).ok());
}

#[test]
fn random_algorithm_has_a_portable_known_sequence() {
    let mut random = Random::new(0);
    assert_eq!(random.next_u64(), 0xe220_a839_7b1d_cdaf);
    assert_eq!(random.next_u64(), 0x6e78_9e6a_a1b9_65f4);
    assert_eq!(random.next_u64(), 0x06c4_5d18_8009_454f);
}

#[test]
fn bounded_random_supports_singletons_and_large_bounds() {
    let mut random = Random::new(1);
    for bound in [1, 3, 1_000, u64::MAX] {
        let limit = NonZeroU64::new(bound).unwrap();
        for _ in 0..1_000 {
            assert!(random.below(limit) < bound);
        }
    }
    assert_eq!(random.below(NonZeroU64::new(1).unwrap()), 0);
}
