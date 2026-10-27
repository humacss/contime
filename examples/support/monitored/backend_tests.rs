#[path = "../../cases/counter/mod.rs"]
mod counter_domain;

use super::{backend::*, Options, Time};
use contime_monitor::WorkerMonitor;
use contime_monitor::WorkerStatusSource;
use counter_domain::{model::CounterChanged, setup::CounterExample};
use std::time::Duration;
#[test]
fn manual_monitor_reads_actual_worker_without_consuming_queued_work() {
    let backend = Manual::<CounterExample>::new(&Options::default()).unwrap();
    backend.submit(vec![CounterChanged { id: 1, time: Time(1), targets: vec![1, 2], value: 1 }]).unwrap();
    backend.advance(Time(100_000_000)).unwrap();
    let mut monitor = WorkerMonitor::attach(&backend);
    monitor.begin(Duration::ZERO).unwrap();
    let reading = monitor.poll(Duration::ZERO).unwrap().unwrap();
    assert_eq!(reading.workers[0].history_stores, 0);
    assert_eq!(reading.workers[0].processing_target, Time(0));
    assert_eq!(reading.workers[0].counters.unwrap().inserted_applications, 0);
    while backend.process().unwrap() {}
    monitor.begin(Duration::from_secs(1)).unwrap();
    let reading = monitor.poll(Duration::from_secs(1)).unwrap().unwrap();
    let worker = &reading.workers[0];
    assert_eq!(worker.history_stores, 2);
    assert_eq!(worker.processing_target, Time(100_000_000));
    assert_eq!(worker.pending_histories, 0);
    assert_eq!(worker.counters.unwrap().inserted_applications, 2);
    assert_eq!(worker.counters.unwrap().replay_applications, Some(2));
}
#[test]
fn manual_admission_reserves_queue_space_for_retention_and_query() {
    let backend = Manual::<CounterExample>::new(&Options { retention_ms: 20, ..Options::default() }).unwrap();
    let mut id = 0;
    while backend.can_submit() {
        id += 1;
        let time = Time(id * 100_000_000);
        backend.submit(vec![CounterChanged { id: id.into(), time, targets: vec![1], value: 1 }]).unwrap();
        backend.advance(time).unwrap();
    }
    let (sender, received) = crossbeam_channel::unbounded();
    backend.query(Time(id * 100_000_000), 1, sender).unwrap();
    assert!(received.try_recv().is_err());
    while backend.process().unwrap() {}
    backend.check_rejections().unwrap();
    let states = received.try_recv().unwrap();
    assert_eq!(states[0].events, id);
    let status = backend.request_worker_status().unwrap().recv().unwrap();
    assert_eq!(status.admission_horizon, Time(id * 100_000_000 - 20_000_000));
    assert_eq!(status.pending_prune_passes, 0);
    assert!(backend.can_submit());
}
