use super::*;
use contime::contime_monitor::{WorkerMonitor, WorkerReading};
use contime::{checkpoints::CheckpointConfig, ConTime, ConTimeConfig, Placement};
use model::{Counter, CounterChanged, Time};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
type Runtime = ConTime<CounterChanged, Counter, ()>;

#[test]
fn shared_hash_host_verifies_fixed_and_maximum_load_with_variable_fanout_and_retention() {
    for maximum in [false, true] {
        let options = monitored::Options {
            workers: 2,
            snapshots: 8,
            targets: 3,
            average_scaled: Some(1_500),
            snapshot_skew: 90,
            rate: 4_000,
            maximum,
            duration: Some(Duration::from_millis(150)),
            headless: true,
            retention_ms: 1,
            checkpoint_interval: 2,
            query_hz: 100,
            ..monitored::Options::default()
        };
        let summary =
            monitored::run_with_options::<setup::HashExample, _, _>(options, generation::events, monitored::Delivery::ReverseGroups)
                .unwrap();
        assert!(summary.events > 0);
        assert_eq!(summary.completed, summary.events);
        assert!(summary.applications >= summary.events);
        assert!(summary.applications <= summary.events * 3);
        assert!(summary.queries > 1, "hashes must be checked during the run as well as at shutdown");
        if maximum {
            assert!(summary.reversed > 0);
        }
    }
}
fn settled(runtime: &Runtime, count: u128, target: Time) -> Arc<WorkerReading<Time>> {
    let clock = Instant::now();
    let mut monitor = WorkerMonitor::attach(runtime);
    loop {
        monitor.begin(clock.elapsed()).unwrap();
        loop {
            if let Some(reading) = monitor.poll(clock.elapsed()).unwrap() {
                let inserted: u128 = reading.workers.iter().map(|w| w.counters.unwrap().inserted_applications as u128).sum();
                if inserted == count && reading.workers.iter().all(|w| w.pending_histories == 0 && w.processing_target >= target) {
                    return reading;
                }
                break;
            }
            assert!(clock.elapsed() < Duration::from_secs(3));
            std::thread::yield_now();
        }
        assert!(clock.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn simulator_hashes_repair_materialized_late_input_and_survive_pruning() {
    let options = monitored::Options { snapshots: 2, targets: 2, ..monitored::Options::default() };
    let events: Vec<_> = generation::events(options, 0, 16, Time(100)).collect();
    let mut expected = Counter::default();
    for event in &events {
        expected.apply_change(event);
    }
    let runtime = Runtime::start(
        ConTimeConfig {
            router_count: 1,
            worker_count: 2,
            placement: Placement::default(),
            history_retention: Time(64),
            pruning_interval: Duration::from_millis(1),
            checkpoints: CheckpointConfig { interval: 2 },
        },
        (),
    )
    .unwrap();
    runtime.apply(events[8..].iter().rev().cloned()).unwrap();
    runtime.advance_to(Time(100)).unwrap();
    settled(&runtime, 16, Time(100)); // The newer eight events really have replayed.
    runtime.apply(events[..8].iter().rev().cloned()).unwrap();
    let repaired = settled(&runtime, 32, Time(100));
    assert!(repaired.workers.iter().map(|w| w.counters.unwrap().replay_applications.unwrap()).sum::<u64>() > 32);
    let queried = runtime.query_at(Time(100), [1, 2]).unwrap();
    assert_eq!(queried.len(), 2);
    for state in queried {
        assert_eq!((state.event_count, state.value, state.hash), (16, expected.value, expected.hash));
    }
    runtime.advance_to(Time(200)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let reading = settled(&runtime, 32, Time(200));
        if reading.workers.iter().all(|w| w.admission_horizon >= Time(136) && w.pending_prune_passes == 0) {
            break;
        }
        assert!(Instant::now() < deadline);
    }
    for state in runtime.query_at(Time(200), [1, 2]).unwrap() {
        assert_eq!((state.event_count, state.value, state.hash), (16, expected.value, expected.hash));
    }
    assert!(runtime.errors().try_iter().next().is_none());
    runtime.shutdown();
}
struct WrongDigest;
impl definition::Example for WrongDigest {
    type Event = CounterChanged;
    type State = Counter;
    const TITLE: &'static str = "Wrong reference";
    const VALUE_LABEL: &'static str = "Total";
    fn event(id: u128, time: Time, targets: Vec<u128>, value: i64) -> CounterChanged {
        CounterChanged { event_id: id, time, counter_ids: targets, delta: value }
    }
    fn id(state: &Counter) -> u128 {
        state.counter_id
    }
    fn count(state: &Counter) -> u64 {
        state.event_count
    }
    fn value(state: &Counter) -> i64 {
        state.value
    }
    fn hash(state: &Counter) -> Option<[u8; 32]> {
        Some(state.hash)
    }
    fn record(state: &mut Counter, event: &CounterChanged) {
        state.apply_change(event);
        state.hash[31] ^= 1; // Counts and totals match; only the full digest differs.
    }
}
#[test]
fn shared_validation_rejects_a_full_digest_mismatch_even_when_counts_and_values_match() {
    let options = monitored::Options {
        snapshots: 1,
        rate: 1_000,
        maximum: false,
        headless: true,
        duration: Some(Duration::from_millis(30)),
        ..monitored::Options::default()
    };
    let result = monitored::run_with_options::<WrongDigest, _, _>(options, generation::events, monitored::Delivery::ReverseGroups);
    assert!(result.unwrap_err().to_string().contains("full digest"));
}

#[test]
fn low_fixed_rate_still_delivers_reversed_pairs() {
    let options = monitored::Options {
        snapshots: 4,
        targets: 1,
        rate: 100,
        maximum: false,
        headless: true,
        duration: Some(Duration::from_millis(150)),
        ..monitored::Options::default()
    };
    let summary =
        monitored::run_with_options::<setup::HashExample, _, _>(options, generation::events, monitored::Delivery::ReverseGroups).unwrap();
    assert!(summary.reversed > 0, "100 events/s must not degenerate to forward delivery");
}

#[test]
fn bounded_hash_windows_validate_partial_groups_and_repeated_single_event_prefixes() {
    for source_limit in [1, 4] {
        let options = monitored::Options {
            workers: 2,
            snapshots: 2,
            targets: 2,
            source_limit: Some(source_limit),
            rate: 1_000,
            maximum: false,
            headless: true,
            duration: Some(Duration::from_millis(100)),
            query_hz: 1_000,
            retention_ms: 1,
            checkpoint_interval: 2,
            ..monitored::Options::default()
        };
        let summary =
            monitored::run_with_options::<setup::HashExample, _, _>(options, generation::events, monitored::Delivery::ReverseGroups)
                .unwrap();
        assert!(summary.events > 1);
        assert_eq!(summary.completed, summary.events);
        assert_eq!(summary.applications, summary.events * 2);
        assert!(summary.queries > 1);
        if source_limit == 4 {
            assert!(summary.reversed > 0);
        }
    }
}

#[cfg(feature = "manual")]
#[test]
fn manual_hash_uses_worker_monitor_feedback_and_reversed_simulator_delivery() {
    for maximum in [false, true] {
        let options = monitored::Options {
            snapshots: 8,
            targets: 3,
            average_scaled: Some(1500),
            snapshot_skew: 90,
            rate: 4000,
            maximum,
            retention_ms: 1,
            checkpoint_interval: 2,
            query_hz: 100,
            headless: true,
            duration: Some(Duration::from_millis(150)),
            ..monitored::Options::default()
        };
        let summary =
            monitored::run_manual_with_options::<setup::HashExample, _, _>(options, generation::events, monitored::Delivery::ReverseGroups)
                .unwrap();
        assert!(summary.events > 0);
        assert_eq!(summary.events, summary.completed);
        assert!(summary.reversed > 0);
        assert!(summary.queries > 1);
    }
}
