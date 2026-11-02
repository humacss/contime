use super::*;
use crate::support::monitored::{backend, Options};
use contime::contime_monitor::WorkerMonitor;
use std::time::Duration;
#[test]
fn late_payment_changes_provisional_winner_and_retries_are_idempotent() {
    scenario(10000);
}
#[test]
fn short_retention_keeps_late_payments_and_retries_replayable() {
    scenario(1);
}
fn scenario(retention_ms: u64) {
    let config = Config { manual: true, retention_ms, checkpoint_interval: 1, ..Default::default() };
    let backend = backend::Manual::<crate::accounts_example::setup::AccountExample>::new(&Options {
        snapshots: 1,
        retention_ms,
        checkpoint_interval: 1,
        ..Default::default()
    })
    .unwrap();
    let mut driver = Driver::new(config);
    let mut monitor = WorkerMonitor::attach(&backend);
    let mut saw_initial_b = false;
    for ms in 0..9000 {
        let done = driver.step(&backend, &mut monitor, Duration::from_millis(ms), ms >= 6000).unwrap();
        let report = driver.report();
        report.validate().unwrap();
        saw_initial_b |= report.reconciled.iter().any(|d| d.id == 3 && d.accepted);
        if done {
            break;
        }
    }
    let report = driver.report();
    assert!(report.done, "final report: {report:?}");
    if retention_ms >= 10000 {
        assert!(saw_initial_b, "initial provisional winner not visible: {report:?}");
    }
    assert_eq!(report.retries, 2);
    assert_eq!(report.submitted, 4);
    assert_eq!(report.balance, 40);
    assert_eq!(report.arrival_balance, 60);
    assert!(report.reconciled.iter().find(|d| d.id == 2).unwrap().accepted);
    assert!(!report.reconciled.iter().find(|d| d.id == 3).unwrap().accepted);
    assert!(!report.arrival_order.iter().find(|d| d.id == 2).unwrap().accepted);
    assert!(report.arrival_order.iter().find(|d| d.id == 3).unwrap().accepted);
    drop(monitor);
    backend.shutdown().unwrap();
}
#[test]
fn early_stop_discards_future_previews() {
    let backend =
        backend::Manual::<crate::accounts_example::setup::AccountExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
    let mut driver = Driver::new(Config { manual: true, ..Default::default() });
    let mut monitor = WorkerMonitor::attach(&backend);
    driver.step(&backend, &mut monitor, Duration::ZERO, false).unwrap();
    assert!(!driver.report().deliveries.is_empty());
    for ms in 1..500 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), true).unwrap() {
            break;
        }
    }
    assert!(driver.done);
    assert_eq!(driver.submitted, 0);
    drop(monitor);
    backend.shutdown().unwrap();
}

#[test]
fn long_host_stall_preserves_ungenerated_transactions_across_retention() {
    let backend =
        backend::Manual::<crate::accounts_example::setup::AccountExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
    let mut driver = Driver::new(Config { manual: true, ..Default::default() });
    let mut monitor = WorkerMonitor::attach(&backend);
    driver.step(&backend, &mut monitor, Duration::ZERO, false).unwrap();
    for ms in 100000..106000 {
        let done = driver.step(&backend, &mut monitor, Duration::from_millis(ms), ms >= 104000).unwrap();
        driver.report().validate().unwrap();
        if done {
            break;
        }
    }
    assert!(driver.done);
    assert!(driver.submitted >= 52);
    drop(monitor);
    backend.shutdown().unwrap();
}

#[test]
fn same_turn_deliveries_preserve_transport_order_in_the_comparison() {
    let backend =
        backend::Manual::<crate::accounts_example::setup::AccountExample>::new(&Options { snapshots: 1, ..Default::default() }).unwrap();
    let mut driver = Driver::new(Config { manual: true, ..Default::default() });
    let mut monitor = WorkerMonitor::attach(&backend);
    driver.step(&backend, &mut monitor, Duration::ZERO, false).unwrap();
    driver.step(&backend, &mut monitor, Duration::from_secs(6), false).unwrap();
    for ms in 6001..7000 {
        if driver.step(&backend, &mut monitor, Duration::from_millis(ms), true).unwrap() {
            break;
        }
    }
    let report = driver.report();
    assert!(report.done);
    assert_eq!((report.balance, report.arrival_balance), (40, 60));
    drop(monitor);
    backend.shutdown().unwrap();
}
