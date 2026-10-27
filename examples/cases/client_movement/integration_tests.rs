//! Real execution regressions: materialized path correction, duplicate inputs, pruning.
use crate::{
    model::{Direction, Motion, MovementInput, Time, SECOND},
    setup::MovementExample,
    support::monitored::{
        backend::{self, Backend},
        Options,
    },
};
use contime_monitor::WorkerMonitor;
use std::time::{Duration, Instant};
fn confirmed<B: Backend<MovementExample>>(backend: &B, count: u64, time: Time) {
    let mut monitor = WorkerMonitor::attach(backend);
    let start = Instant::now();
    let mut collecting = false;
    loop {
        assert!(start.elapsed() < Duration::from_secs(5), "worker status confirmation timed out");
        backend.process().unwrap();
        if !collecting {
            monitor.begin(start.elapsed()).unwrap();
            collecting = true;
        }
        if let Some(reading) = monitor.poll(start.elapsed()).unwrap() {
            collecting = false;
            if reading.workers.iter().map(|w| w.counters.unwrap().inserted_applications).sum::<u64>() == count
                && reading.workers.iter().all(|w| w.processing_target >= time && w.pending_histories == 0)
            {
                return;
            }
        }
        std::thread::yield_now();
    }
}
fn position<B: Backend<MovementExample>>(backend: &B, time: Time) -> ([i128; 2], u64) {
    let (sender, receiver) = crossbeam_channel::unbounded();
    backend.query(time, 1, sender).unwrap();
    let start = Instant::now();
    let mut states = Vec::new();
    loop {
        backend.process().unwrap();
        match receiver.try_recv() {
            Ok(batch) => states.extend(batch),
            Err(crossbeam_channel::TryRecvError::Empty) => {
                assert!(start.elapsed() < Duration::from_secs(5));
                std::thread::yield_now();
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => break,
        }
    }
    assert_eq!(states.len(), 1);
    (states[0].position_at(time), states[0].events)
}
fn verify<B: Backend<MovementExample>>(backend: B) {
    let input = |id, seconds: u64, direction| MovementInput {
        id,
        client_id: 1,
        time: Time(seconds * 1_000_000_000 + 1),
        direction,
        motion: Motion { client_spacing: 9, movement_speed: 10 },
    };
    backend.submit(vec![input(1, 0, Direction::East), input(3, 2, Direction::Stop)]).unwrap();
    backend.advance(Time(3_000_000_001)).unwrap();
    confirmed(&backend, 2, Time(3_000_000_001));
    assert_eq!(position(&backend, Time(3_000_000_001)), ([20 * SECOND, 0], 2));
    backend.submit(vec![input(2, 1, Direction::North), input(2, 1, Direction::North)]).unwrap();
    confirmed(&backend, 3, Time(3_000_000_001));
    assert_eq!(position(&backend, Time(3_000_000_001)), ([10 * SECOND, 10 * SECOND], 3));
    backend.advance(Time(5_000_000_001)).unwrap();
    confirmed(&backend, 3, Time(5_000_000_001));
    assert_eq!(position(&backend, Time(5_000_000_001)), ([10 * SECOND, 10 * SECOND], 3));
    backend.check_rejections().unwrap();
    backend.shutdown().unwrap();
}
#[test]
fn runtime_repairs_an_old_turn_after_queries_and_retains_the_correct_path() {
    let options = Options { snapshots: 1, retention_ms: 2000, checkpoint_interval: 1, ..Default::default() };
    verify(backend::runtime::<MovementExample>(&options).unwrap());
}
#[cfg(feature = "manual")]
#[test]
fn manual_repairs_an_old_turn_after_queries_and_retains_the_correct_path() {
    let options = Options { snapshots: 1, retention_ms: 2000, checkpoint_interval: 1, ..Default::default() };
    verify(backend::Manual::<MovementExample>::new(&options).unwrap());
}
