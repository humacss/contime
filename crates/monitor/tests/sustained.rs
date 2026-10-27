#[allow(dead_code)]
#[path = "../benches/support/pipeline.rs"]
mod pipeline;
#[path = "../benches/support/sustained.rs"]
mod sustained;
use pipeline::Mode;
use std::time::Duration;
use sustained::{Config, Harness};

#[test]
fn periodic_monitoring_and_disabled_runs_validate_across_many_intervals() {
    for workers in [1, 4] {
        for fanout in [1, 3] {
            for mode in [Mode::Off, Mode::PerEvent] {
                let config = Config {
                    workers,
                    fanout,
                    snapshots: 32,
                    query_interval: Duration::from_millis(20),
                    sample_interval: Duration::from_millis(10),
                    maximum_pending: 1024,
                    retention: 512,
                };
                let mut harness = Harness::new(config, mode);
                let mut tick = 0;
                let mut clock = || {
                    tick += 5;
                    Duration::from_millis(tick)
                };
                harness.run_events(4099, &mut clock);
                harness.settle(&mut clock);
                let stats = harness.stats();
                assert_eq!(stats.events, 4099);
                assert_eq!(stats.applications, 4099 * fanout as u64);
                assert_eq!(stats.queried_events, 4099);
                assert!(stats.periodic_queries >= 3);
                assert!(stats.sample_ticks >= 5);
                assert!(stats.maximum_pending <= 1024);
                assert!(stats.history_len <= 120);
                match harness.view() {
                    Some(view) => {
                        assert_eq!(view.submitted, view.confirmed);
                        assert_eq!(view.confirmed.events, 4099);
                        assert_eq!(view.outstanding.events, 0);
                    }
                    None => assert_eq!(mode, Mode::Off),
                }
                harness.shutdown();
            }
        }
    }
}

#[test]
fn capacity_guard_is_independent_of_monitor_and_runtime_persists_across_segments() {
    for mode in [Mode::Off, Mode::PerEvent] {
        let mut harness = Harness::new(Config { maximum_pending: 512, retention: 512, ..Config::default() }, mode);
        let mut clock = || Duration::ZERO;
        harness.run_events(2048, &mut clock);
        harness.settle(&mut clock);
        let first = harness.stats();
        harness.run_events(2051, &mut clock);
        harness.settle(&mut clock);
        let second = harness.stats();
        assert_eq!(first.events, 2048);
        assert_eq!(second.events, 4099);
        assert_eq!(second.queried_events, 4099);
        assert!(second.capacity_queries >= 8);
        assert_eq!(second.periodic_queries, 0);
        assert_eq!(second.maximum_pending, 512);
        harness.shutdown();
    }
}

#[test]
fn periodic_deadlines_keep_their_phase_and_skip_missed_ticks() {
    let interval = Duration::from_millis(50);
    assert_eq!(sustained::next_tick(Duration::ZERO, interval), Duration::from_millis(50));
    assert_eq!(sustained::next_tick(Duration::from_millis(51), interval), Duration::from_millis(100));
    assert_eq!(sustained::next_tick(Duration::from_millis(180), interval), Duration::from_millis(200));
}

#[test]
fn monitor_history_rolls_over_without_resetting_runtime_or_cumulative_state() {
    let mut harness = Harness::new(Config { snapshots: 8, retention: 512, maximum_pending: 512, ..Config::default() }, Mode::PerEvent);
    let mut tick = 0;
    let mut clock = || {
        tick += 100;
        Duration::from_millis(tick)
    };
    for _ in 0..32 {
        harness.run_events(513, &mut clock);
        harness.settle(&mut clock);
    }
    let stats = harness.stats();
    assert!(stats.sample_ticks > 120);
    assert_eq!(stats.history_len, 120);
    assert!(stats.pruned_horizon > 0, "Runtime must have completed real retention");
    assert_eq!(stats.events, 32 * 513);
    let view = harness.view().unwrap();
    assert_eq!(view.submitted, view.confirmed);
    assert_eq!(view.confirmed.events, 32 * 513);
    assert!(view.average_confirmed.events_per_second.is_finite());
    harness.shutdown();
}
