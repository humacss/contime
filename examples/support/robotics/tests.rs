use super::*;
use crate::support::monitored::{backend, Options};
use contime::contime_monitor::WorkerMonitor;
use std::time::Duration;
#[test]
fn reversed_camera_arrivals_replay_from_capture_time_and_change_the_proposal() {
    run(10000);
}
#[test]
fn short_retention_preserves_camera_inputs_and_forward_replay() {
    run(1);
}
fn run(retention_ms: u64) {
    let backend = backend::Manual::<crate::robotics_example::setup::RoboticsExample>::new(&Options {
        snapshots: 1,
        retention_ms,
        checkpoint_interval: 1,
        ..Default::default()
    })
    .unwrap();
    let mut driver = Driver::new(Config { manual: true, retention_ms, checkpoint_interval: 1, ..Default::default() });
    let mut monitor = WorkerMonitor::attach(&backend);
    let mut backtrack = false;
    for ms in 0..10000 {
        let done = driver.step(&backend, &mut monitor, Duration::from_millis(ms), ms >= 6500).unwrap();
        let report = driver.report();
        report.validate().unwrap();
        backtrack |= report.historical.proposal == "Backtrack to aisle";
        if done {
            break;
        }
    }
    let report = driver.report();
    assert!(report.done);
    assert_eq!(report.historical.observations, 2);
    assert_eq!(report.current.observations, 2);
    assert!((report.historical.pose.x - report.truth.x).abs() < (report.current.pose.x - report.truth.x).abs());
    let mut arrived: Vec<_> = report.observations.iter().filter(|o| o.arrived_ms.is_some()).collect();
    arrived.sort_by_key(|o| o.arrived_ms);
    assert_eq!(arrived.iter().map(|o| o.capture_ms).collect::<Vec<_>>(), vec![5000, 4000]);
    if retention_ms >= 10000 {
        assert!(backtrack);
    }
    drop(monitor);
    backend.shutdown().unwrap();
}
#[test]
fn pause_and_early_stop_do_not_admit_future_observations() {
    let backend =
        backend::Manual::<crate::robotics_example::setup::RoboticsExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
    let mut driver = Driver::new(Config { manual: true, ..Default::default() });
    let mut monitor = WorkerMonitor::attach(&backend);
    for _ in 0..100 {
        driver.step(&backend, &mut monitor, Duration::ZERO, false).unwrap();
    }
    assert_eq!(driver.submitted, 0);
    for ms in 1..500 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), true).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    assert_eq!(driver.report().historical.observations, 0);
    drop(monitor);
    backend.shutdown().unwrap();
}

#[test]
fn catch_up_does_not_prune_not_yet_generated_motion_or_late_camera_results() {
    let backend =
        backend::Manual::<crate::robotics_example::setup::RoboticsExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
    let mut driver = Driver::new(Config { manual: true, ..Default::default() });
    let mut monitor = WorkerMonitor::attach(&backend);
    driver.step(&backend, &mut monitor, Duration::ZERO, false).unwrap();
    for ms in 100000..110000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), ms >= 108000).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    assert!(driver.report().historical.observations > 40);
    driver.report().validate().unwrap();
    drop(monitor);
    backend.shutdown().unwrap();
}
#[test]
fn coincident_events_use_motion_then_observation_then_planning_even_if_sent_in_reverse() {
    use crate::{
        definition::Time,
        robotics_example::model::{Pose, Reading, SensorEvent},
        support::monitored::backend::Backend,
    };
    let backend =
        backend::Manual::<crate::robotics_example::setup::RoboticsExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
    let event = |id, reading| SensorEvent { id, robot_id: 1, time: Time(1), reading };
    backend
        .submit(vec![
            event(1, Reading::Plan),
            event(2, Reading::Camera(Pose { x: 4.0, y: 0.0, heading: 0.0 })),
            event(3, Reading::Motion { distance: 1.0, turn: 0.0 }),
        ])
        .unwrap();
    backend.advance(Time(1)).unwrap();
    let (send, receive) = crossbeam_channel::unbounded();
    backend.query(Time(1), 1, send).unwrap();
    for _ in 0..100 {
        backend.process().unwrap();
    }
    let state = receive.try_recv().unwrap().remove(0);
    assert!((state.pose.x - 3.55).abs() < 1e-12);
    assert_eq!(state.events, 3);
    backend.shutdown().unwrap();
}

#[test]
fn submillisecond_camera_delivery_keeps_distinct_trajectory_samples() {
    let backend =
        backend::Manual::<crate::robotics_example::setup::RoboticsExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
    let mut driver = Driver::new(Config { manual: true, ..Default::default() });
    let mut monitor = WorkerMonitor::attach(&backend);
    for ms in 0..9000 {
        let done = driver.step(&backend, &mut monitor, Duration::from_micros(ms * 1000 + 37), ms >= 6500).unwrap();
        driver.report().validate().unwrap();
        if done {
            break;
        }
    }
    assert!(driver.done);
    drop(monitor);
    backend.shutdown().unwrap();
}

#[test]
fn duration_stop_after_a_host_stall_catches_up_all_captured_inputs() {
    let backend =
        backend::Manual::<crate::robotics_example::setup::RoboticsExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
    let mut driver = Driver::new(Config { manual: true, duration: Some(Duration::from_secs(6)), ..Default::default() });
    let mut monitor = WorkerMonitor::attach(&backend);
    driver.step(&backend, &mut monitor, Duration::ZERO, false).unwrap();
    for ms in 7000..10000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), false).unwrap() {
            break;
        }
    }
    let report = driver.report();
    assert!(report.done);
    assert_eq!(report.historical.observations, 2);
    assert_eq!(report.current.observations, 2);
    assert_eq!(report.historical.trail.last().unwrap().time_ns, 6_000_000_001);
    assert!((report.truth.x - 6.0).abs() < 1e-12);
    drop(monitor);
    backend.shutdown().unwrap();
}
