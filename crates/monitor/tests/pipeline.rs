#[allow(dead_code)]
#[path = "../benches/support/pipeline.rs"]
mod pipeline;
use contime_monitor::{Monitor, Totals};
use pipeline::{Harness, Mode, Time};
use std::time::Duration;

#[test]
fn monitored_runtime_matches_unmonitored_for_reordered_events_and_fanout() {
    for workers in [1, 4] {
        for fanout in [1, 3] {
            for mode in Mode::ALL {
                // A remainder checks both complete and incomplete recording batches.
                let mut harness = Harness::new(workers, 128, 4099, fanout);
                let report = harness.run(mode);
                assert_eq!(report.totals, Totals { events: 4099, applications: 4099 * fanout as u64 });
                match report.metrics {
                    None => assert_eq!(mode, Mode::Off),
                    Some(view) => {
                        assert_eq!(view.submitted, report.totals);
                        assert_eq!(view.confirmed, report.totals);
                        assert_eq!(view.outstanding, Totals::default());
                        assert_eq!(view.active_query, None);
                        assert!(view.last_query.is_some());
                    }
                }
                harness.shutdown();
            }
        }
    }
}

#[test]
fn runtime_historical_query_confirms_only_its_captured_prefix() {
    let mut harness = Harness::new(2, 32, 256, 3);
    let mut monitor = Monitor::default();
    monitor.sample(Duration::ZERO).unwrap();
    let (response, inserted) = crossbeam_channel::unbounded();
    let inputs = std::mem::take(&mut harness.events);
    let (earlier, later): (Vec<_>, Vec<_>) = inputs.into_iter().partition(|event| event.sequence <= 128);
    harness.runtime.send(earlier, response.clone()).unwrap();
    monitor.record_submission(Totals { events: 128, applications: 384 }).unwrap();
    let token = monitor.start_query(Duration::from_secs(1)).unwrap();
    harness.runtime.send(later, response).unwrap();
    assert!(inserted.recv_timeout(Duration::from_secs(10)).is_err_and(|e| e == crossbeam_channel::RecvTimeoutError::Disconnected));
    monitor.record_submission(Totals { events: 128, applications: 384 }).unwrap();
    let states = harness.runtime.query_at(Time(128), 1..=32).unwrap();
    assert_eq!(states.iter().map(|state| state.count).sum::<u64>(), 384);
    monitor.complete_query(token, Duration::from_secs(2)).unwrap();
    let view = monitor.sample(Duration::from_secs(2)).unwrap();
    assert_eq!(view.confirmed, Totals { events: 128, applications: 384 });
    assert_eq!(view.outstanding, Totals { events: 128, applications: 384 });
    let token = monitor.start_query(Duration::from_secs(3)).unwrap();
    let states = harness.runtime.query_at(Time(256), 1..=32).unwrap();
    harness.validate(&states);
    monitor.complete_query(token, Duration::from_secs(4)).unwrap();
    assert_eq!(monitor.snapshot(Duration::from_secs(4)).unwrap().outstanding, Totals::default());
    harness.shutdown();
}

#[test]
fn single_target_fixture_contains_late_arrivals_in_every_snapshot_history() {
    let harness = Harness::new(1, 128, 4096, 1);
    let mut last = [0u64; 128];
    let mut late = [false; 128];
    for event in &harness.events {
        let index = (event.sequence as usize - 1) % 128;
        late[index] |= last[index] > event.sequence;
        last[index] = event.sequence;
    }
    assert!(late.into_iter().all(|value| value));
    harness.shutdown();
}
