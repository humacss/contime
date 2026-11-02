use super::*;
use crate::support::monitored::{backend, Options};
fn options(config: &Config) -> Options {
    Options {
        snapshots: config.services,
        workers: config.workers,
        retention_ms: config.retention_ms,
        checkpoint_interval: config.checkpoint_interval,
        ..Default::default()
    }
}
fn config() -> Config {
    Config { services: 1, manual: true, headless: true, duration: Some(Duration::from_secs(4)), ..Default::default() }
}
#[test]
fn future_preview_does_not_submit_early_and_stop_discards_unoccurred_actions() {
    let config = Config { services: 5, duration: None, headless: false, min_delay_ms: 0, latency_ms: 0, jitter_ms: 0, ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    let mut monitor = WorkerMonitor::attach(&backend);
    for ms in 0..=50 {
        driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap();
    }
    let preview = driver.report();
    assert_eq!(preview.inputs.len(), 15, "preview the next three one-second ticks for all services");
    assert!(preview.inputs.iter().all(|input| input.original_ms > preview.clock_ms && input.arrived_ms.is_none()));
    assert_eq!(driver.submitted, 5, "only initialization inputs have been submitted");
    preview.validate().unwrap();
    for ms in 51..=1000 {
        driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap();
    }
    for ms in 1001..5000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), true).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    assert_eq!(driver.submitted, 10, "only the first tick occurred before stop");
    assert_eq!(driver.in_flight(), 0);
    assert!(driver.report().inputs.iter().all(|input| input.original_ms <= 1000 && input.arrived_ms.is_some()));
    driver.report().validate().unwrap();
    drop(monitor);
    backend.shutdown().unwrap();
}
#[test]
fn late_down_after_recovery_repairs_outage_and_current_status() {
    let config = config();
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    driver.next_tick = 1000;
    let event = |id, ms: u64, status| StatusChanged { id, service_id: 1, time: Time(ms * 1_000_000 + 1), status };
    driver.waiting.insert((2_200_000_001, 2), event(2, 1000, Status::Down));
    driver.waiting.insert((1_500_000_001, 3), event(3, 1500, Status::Healthy));
    let mut monitor = WorkerMonitor::attach(&backend);
    for ms in 0..7000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), ms >= 3000).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    let report = driver.report();
    report.validate().unwrap();
    let row = &report.services[0];
    assert_eq!(row.reconciled.status, report::Status::Healthy);
    assert_eq!(row.reconciled.downtime_ms, 500);
    assert_eq!(row.arrival_order.status, report::Status::Down);
    assert!(row.arrival_order.downtime_ms >= 799);
    assert_eq!(report.late, 1);
    assert!(report.inputs.iter().find(|e| e.id == 2).unwrap().late);
    assert_eq!(driver.completed, driver.submitted);
    let before = report.clone();
    driver.step(&backend, &mut monitor, Duration::from_secs(20), true).unwrap();
    let after = driver.report();
    assert_eq!(before.services, after.services);
    assert_eq!(before.now_ms, after.now_ms);
    assert_eq!(after.clock_ms, 20_000);
    drop(monitor);
    backend.shutdown().unwrap();
}
#[test]
fn short_retention_preserves_delayed_inputs_and_drains_without_new_generation() {
    let config = Config { retention_ms: 1, checkpoint_interval: 1, duration: Some(Duration::from_secs(12)), ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    let mut monitor = WorkerMonitor::attach(&backend);
    let mut stop_count = None;
    for ms in 0..20000 {
        driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap();
        if ms == 12000 {
            stop_count = Some(driver.sequence);
        }
        if driver.done {
            break;
        }
    }
    assert!(driver.done);
    assert_eq!(driver.sequence, stop_count.unwrap());
    assert_eq!(driver.in_flight(), 0);
    assert!(driver.late() > 0);
    driver.report().validate().unwrap();
    assert_eq!(driver.completed, driver.submitted);
    drop(monitor);
    backend.shutdown().unwrap();
}
#[test]
fn duplicate_identity_is_ignored_by_real_backend_and_reference() {
    let config = config();
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let event = StatusChanged { id: 7, service_id: 1, time: Time(1), status: Status::Down };
    backend.submit(vec![event.clone(), event.clone()]).unwrap();
    backend.advance(Time(1_000_001)).unwrap();
    for _ in 0..100 {
        backend.process().unwrap();
    }
    backend.check_rejections().unwrap();
    let (send, recv) = crossbeam_channel::unbounded();
    backend.query(Time(1_000_001), 1, send).unwrap();
    for _ in 0..100 {
        backend.process().unwrap();
    }
    let states: Vec<_> = recv.try_iter().flatten().collect();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].events, 1);
    assert_eq!(states[0].totals_at(Time(1_000_001)).1, 1_000_000);
    let mut reference = Reference::new(1);
    reference.insert(event.clone());
    reference.insert(event);
    assert_eq!(reference.states(Time(1_000_001))[0].events, 1);
    backend.shutdown().unwrap();
}
#[test]
fn clock_and_configuration_reject_invalid_input() {
    let config = config();
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    let mut monitor = WorkerMonitor::attach(&backend);
    driver.step(&backend, &mut monitor, Duration::from_millis(10), false).unwrap();
    assert!(driver.step(&backend, &mut monitor, Duration::ZERO, false).is_err());
    assert!(Config::parse(["--services", "0"].map(str::to_owned)).is_err());
    assert!(super::super::config("{\"services\":1,\"latency_ms\":0,\"jitter_ms\":0}").is_ok());
    drop(monitor);
    backend.shutdown().unwrap();
}
#[test]
fn seeded_workload_has_out_of_order_arrivals_and_stays_bounded() {
    let config = Config { services: 5, duration: Some(Duration::from_secs(25)), ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    let mut monitor = WorkerMonitor::attach(&backend);
    for ms in 0..30000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    let report = driver.report();
    report.validate().unwrap();
    assert!(report.late > 0);
    assert!(report.inputs.len() <= report::MAX_INPUTS);
    assert!(report.services[0].reconciled.downtime_ms > 0);
    drop(monitor);
    backend.shutdown().unwrap();
}

#[test]
fn stop_flushes_pending_arrivals_before_finishing_both_views() {
    let config = Config { duration: Some(Duration::from_millis(1200)), ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    let mut monitor = WorkerMonitor::attach(&backend);
    for ms in 0..6000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    let report = driver.report();
    report.validate().unwrap();
    assert!((1200..2000).contains(&report.now_ms));
    assert_eq!(report.services[0].arrival_order.status, report::Status::Down);
    assert_eq!(report.services[0].reconciled.status, report::Status::Down);
    assert_eq!(report.in_flight, 0);
    assert!(report.inputs.iter().all(|e| e.arrived_ms.is_some()));
    drop(monitor);
    backend.shutdown().unwrap();
}

#[test]
fn zero_delay_views_match_and_history_remains_bounded_after_many_changes() {
    let config = Config {
        cadence_ms: 100,
        min_delay_ms: 0,
        latency_ms: 0,
        jitter_ms: 0,
        retention_ms: 1,
        checkpoint_interval: 1,
        duration: Some(Duration::from_secs(40)),
        ..config()
    };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    let mut monitor = WorkerMonitor::attach(&backend);
    for ms in 0..45000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    let report = driver.report();
    report.validate().unwrap();
    let row = &report.services[0];
    assert_eq!(row.reconciled.status, row.arrival_order.status);
    // Delivery is handed off on host turns and can lag its scheduled time by a few ms.
    assert!(row.reconciled.downtime_ms.abs_diff(row.arrival_order.downtime_ms) < 100);
    assert!(driver.reference.states(driver.target)[0].events > 256);
    assert_eq!(driver.reference.states(driver.target)[0].intervals.len(), 256);
    assert!(row.reconciled.intervals.len() <= report::MAX_INTERVALS);
    drop(monitor);
    backend.shutdown().unwrap();
}
#[test]
fn generation_catch_up_is_bounded_per_host_turn() {
    let config = Config { services: 16, ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    driver.generate(1_000_000_000_000).unwrap();
    assert_eq!(driver.next_tick, 64);
    assert_eq!(driver.waiting.len(), 64 * 16);
    backend.shutdown().unwrap();
}

#[test]
fn a_delivery_burst_preserves_handoff_order_for_equal_arrival_timestamps() {
    let config = config();
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    driver.next_tick = 1000;
    driver.waiting.insert((2_600_000_001, 2), StatusChanged { id: 2, service_id: 1, time: Time(1_000_000_001), status: Status::Down });
    driver.waiting.insert((2_000_000_001, 3), StatusChanged { id: 3, service_id: 1, time: Time(2_000_000_001), status: Status::Healthy });
    let mut monitor = WorkerMonitor::attach(&backend);
    for ms in 3000..5000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), true).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    let report = driver.report();
    report.validate().unwrap();
    assert_eq!(report.services[0].reconciled.status, report::Status::Healthy);
    assert_eq!(report.services[0].arrival_order.status, report::Status::Down);
    assert_eq!(report.inputs.iter().map(|e| e.id).collect::<Vec<_>>(), vec![3, 2]);
    drop(monitor);
    backend.shutdown().unwrap();
}

#[test]
fn delivery_clock_remains_live_when_short_retention_stalls_the_query_horizon() {
    let config = Config { min_delay_ms: 50, retention_ms: 1, checkpoint_interval: 1, ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    // Isolate retention from random sampling: recovery arrives while an older outage is in flight.
    driver.next_tick = 1000;
    driver.waiting.insert((2_600_000_001, 2), StatusChanged { id: 2, service_id: 1, time: Time(1_000_000_001), status: Status::Down });
    driver.waiting.insert((2_050_000_001, 3), StatusChanged { id: 3, service_id: 1, time: Time(2_000_000_001), status: Status::Healthy });
    let mut monitor = WorkerMonitor::attach(&backend);
    for ms in 0..=2200 {
        driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap();
    }
    let report = driver.report();
    report.validate().unwrap();
    assert_eq!(report.clock_ms, 2200);
    assert!(report.now_ms <= 1001);
    assert!(report.inputs.iter().any(|input| input.arrived_ms.is_some_and(|at| at >= 2000)));
    assert!(report.inputs.iter().any(|input| input.arrived_ms.is_none() && input.delivery_ms > report.clock_ms));
    drop(monitor);
    backend.shutdown().unwrap();
}
#[test]
fn clock_boundaries_keep_one_nanosecond_application_origin_out_of_reports() {
    let config = config();
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    let mut monitor = WorkerMonitor::attach(&backend);
    let elapsed = Duration::from_nanos(999_999);
    for _ in 0..100 {
        driver.next_status = 0; // Poll the same tiny clock without waiting for the status cadence.
        driver.step(&backend, &mut monitor, elapsed, false).unwrap();
    }
    let report = driver.report();
    report.validate().unwrap();
    assert_eq!(report.services.len(), 1);
    assert_eq!(report.clock_ms, 0);
    assert_eq!(report.now_ms, 0);
    drop(monitor);
    backend.shutdown().unwrap();
}

#[test]
fn stop_flushes_a_minute_of_simulated_latency_without_waiting_a_minute() {
    let config = Config { latency_ms: 60_000, jitter_ms: 0, duration: Some(Duration::from_millis(1100)), ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    let mut monitor = WorkerMonitor::attach(&backend);
    for ms in 0..2000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    let report = driver.report();
    report.validate().unwrap();
    assert!(report.clock_ms < 2000);
    assert_eq!(report.submitted, 2);
    assert_eq!(report.completed, 2);
    assert_eq!(report.in_flight, 0);
    assert_eq!(report.services[0].reconciled.status, report::Status::Down);
    assert_eq!(report.services[0].arrival_order.status, report::Status::Down);
    let input = &report.inputs[0];
    assert_eq!(input.delivery_ms, 61000);
    assert!(input.arrived_ms.unwrap() < 2000);
    drop(monitor);
    backend.shutdown().unwrap();
}

#[test]
fn impossible_pending_load_is_rejected_at_startup_but_short_runs_remain_valid() {
    let overloaded =
        Config { services: 16, cadence_ms: 100, latency_ms: 60_000, jitter_ms: 0, duration: Some(Duration::from_secs(54)), ..config() };
    let error = overloaded.validate().unwrap_err();
    assert!(error.contains("8192 pending changes"), "{error}");
    let short = Config { duration: Some(Duration::from_secs(1)), ..overloaded };
    short.validate().unwrap();
    Config::default().validate().unwrap();
}

#[test]
fn bounded_preview_includes_all_small_queues_and_reserves_room_for_arrivals() {
    let config = Config { services: 5, ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    driver.now = Duration::from_secs(2);
    driver.generate(2_000_000_001).unwrap();
    driver.view.services = (1..=5)
        .map(|id| report::ServiceReport {
            id,
            name: format!("Service {id}"),
            reconciled: Default::default(),
            arrival_order: Default::default(),
        })
        .collect();
    driver.view.now_ms = 2000;
    assert_eq!(driver.waiting.len(), 10);
    let report = driver.report();
    report.validate().unwrap();
    assert_eq!(report.inputs.len(), 10);
    for service in 1..=5 {
        assert_eq!(report.inputs.iter().filter(|input| input.service == service).count(), 2);
    }
    for id in 1000..1128 {
        driver.recent.push_back(report::Input {
            id,
            service: 1,
            status: report::Status::Healthy,
            original_ms: 0,
            delivery_ms: 0,
            arrived_ms: Some(0),
            late: false,
        });
    }
    let report = driver.report();
    assert_eq!(report.inputs.len(), report::MAX_INPUTS);
    assert_eq!(report.inputs.iter().filter(|input| input.arrived_ms.is_none()).count(), 10);
    driver.now = Duration::from_secs(16);
    driver.generate(16_000_000_001).unwrap();
    let report = driver.report();
    report.validate().unwrap();
    assert_eq!(report.inputs.len(), report::MAX_INPUTS);
    assert_eq!(report.inputs.iter().filter(|input| input.arrived_ms.is_none()).count(), report::MAX_INPUTS / 2);
    assert!(report.inputs.iter().any(|input| input.id == 1127));
    backend.shutdown().unwrap();
}

#[test]
fn minimum_delay_is_applied_after_jitter_and_round_trips_across_hosts() {
    let config = Config { services: 5, min_delay_ms: 50, duration: None, headless: false, ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config.clone()).unwrap();
    driver.generate(4_000_000_001).unwrap();
    let delays: Vec<_> = driver.waiting.iter().map(|((due, _), event)| (due - event.time.0) / 1_000_000).collect();
    assert!(delays.iter().all(|delay| *delay >= 50));
    assert!(delays.contains(&50));
    assert!(delays.iter().all(|delay| *delay <= 1600));
    assert!(delays.iter().any(|delay| *delay > 800));
    assert_eq!(Config::parse(config.arguments()).unwrap().unwrap().min_delay_ms, 50);
    assert_eq!(super::super::config(r#"{"min_delay_ms":75}"#).unwrap().min_delay_ms, 75);
    assert_eq!(Config { min_delay_ms: 2000, ..config }.max_latency_ms(), 2000);
    backend.shutdown().unwrap();
}

#[test]
fn default_source_changes_align_to_seconds_while_deliveries_vary_and_reorder() {
    let config = Config { services: 5, ..config() };
    assert_eq!(config.cadence_ms, 1000);
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let mut driver = Driver::new(&backend, config).unwrap();
    driver.generate(4_000_000_001).unwrap();
    assert_eq!(driver.report().cadence_ms, Some(1000));
    let mut overtaken = false;
    for service in 1..=5 {
        let mut changes: Vec<_> = driver
            .waiting
            .iter()
            .filter(|(_, event)| event.service_id == service)
            .map(|((delivery, _), event)| (millis(event.time.0), millis(*delivery)))
            .collect();
        changes.sort_by_key(|(action, _)| *action);
        assert_eq!(changes.iter().map(|(action, _)| *action).collect::<Vec<_>>(), vec![1000, 2000, 3000, 4000]);
        assert!(changes.windows(2).any(|pair| pair[1].1.abs_diff(pair[0].1) != 1000));
        overtaken |= changes.windows(2).any(|pair| pair[1].1 < pair[0].1);
    }
    assert!(overtaken, "recovery must sometimes arrive before an older outage");
    backend.shutdown().unwrap();
}

#[test]
fn seeded_delays_vary_within_each_service_and_status_and_repeat_on_reset() {
    let config = Config { services: 5, ..config() };
    let backend = backend::Manual::<ServiceExample>::new(&options(&config)).unwrap();
    let sample = || {
        let mut driver = Driver::new(&backend, config.clone()).unwrap();
        driver.generate(32_000_000_001).unwrap();
        driver
            .waiting
            .iter()
            .map(|((due, _), event)| (event.id, event.service_id, event.status, (due - event.time.0) / 1_000_000))
            .collect::<Vec<_>>()
    };
    let first = sample();
    assert_eq!(first, sample(), "reset must reproduce the same network schedule");
    for service in 1..=5 {
        for status in [Status::Healthy, Status::Degraded, Status::Down] {
            let delays: std::collections::BTreeSet<_> =
                first.iter().filter(|(_, id, state, _)| *id == service && *state == status).map(|(_, _, _, delay)| *delay).collect();
            assert!(delays.len() >= 3, "delays must not be selected by service or status");
        }
    }
    assert!(first.iter().all(|(_, _, _, delay)| (250..=1600).contains(delay)));
    backend.shutdown().unwrap();
}
