#![cfg(feature = "runtime")]
#[path = "../benches/support/workers.rs"]
mod fixture;
use contime_monitor::WorkerMonitor;
use std::time::{Duration, Instant};
#[test]
fn status_collection_observes_existing_stores_without_advancing_execution() {
    for workers in [1, 4] {
        let runtime = fixture::start(workers);
        let origin = Instant::now();
        let mut monitor = WorkerMonitor::attach(&runtime);
        monitor.begin(origin.elapsed()).unwrap();
        let empty = collect(&mut monitor, origin);
        assert_eq!(empty.workers.len(), workers);
        assert_eq!(empty.workers.iter().map(|s| s.history_stores).sum::<usize>(), 0);
        fixture::send(&runtime, 1, 1024, 128);
        monitor.begin(origin.elapsed()).unwrap();
        let populated = collect(&mut monitor, origin);
        assert_eq!(populated.workers.iter().map(|s| s.history_stores).sum::<usize>(), 128);
        assert!(populated.workers.iter().all(|s| s.processing_target == fixture::Time(0)));
        assert!(populated.workers.iter().map(|s| s.pending_histories).sum::<usize>() > 0);
        let states = runtime.query_at(fixture::Time(1024), 1..=128).unwrap();
        assert_eq!(states.iter().map(|s| s.count).sum::<u64>(), 1024);
        drop(monitor);
        runtime.shutdown();
    }
}
fn collect<S: contime_monitor::WorkerStatusSource<fixture::Time>>(
    monitor: &mut WorkerMonitor<'_, fixture::Time, S>,
    origin: Instant,
) -> std::sync::Arc<contime_monitor::WorkerReading<fixture::Time>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(reading) = monitor.poll(origin.elapsed()).unwrap() {
            return reading;
        }
        assert!(Instant::now() < deadline, "worker collection timed out");
        std::thread::yield_now();
    }
}

#[test]
fn cancelled_collection_does_not_keep_runtime_alive() {
    let runtime = fixture::start(4);
    let mut monitor = WorkerMonitor::attach(&runtime);
    monitor.begin(Duration::ZERO).unwrap();
    monitor.cancel();
    drop(monitor);
    let started = Instant::now();
    runtime.shutdown();
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[cfg(feature = "counters")]
mod counters {
    use super::fixture;
    use contime_runtime::{
        checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event},
        ConTime, ConTimeConfig, Input, Placement,
    };
    use std::time::Duration;
    struct Fanout(u64);
    impl Event for Fanout {
        type Time = fixture::Time;
        fn time(&self) -> Self::Time {
            fixture::Time(self.0)
        }
    }
    impl Input for Fanout {
        fn event_id(&self) -> u128 {
            self.0.into()
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            for id in 1..=3 {
                emit(id);
            }
        }
    }
    impl ApplyEvents<Fanout> for fixture::Counter {
        fn create(_: u128, _: &Fanout) -> Self {
            Self::default()
        }
        fn apply_events(&mut self, batch: ApplyBatch<'_, '_, fixture::Time, Fanout>) {
            self.count += batch.events.count() as u64;
        }
    }
    #[test]
    fn native_counters_count_fanout_replay_and_exclude_queries() {
        for workers in [1, 4] {
            let runtime = ConTime::<Fanout, fixture::Counter, ()>::start(
                ConTimeConfig {
                    worker_count: workers,
                    router_count: 1,
                    placement: Placement::default(),
                    history_retention: fixture::Time(u64::MAX),
                    pruning_interval: Duration::from_secs(60),
                    checkpoints: CheckpointConfig { interval: 128 },
                },
                (),
            )
            .unwrap();
            let send = |events| {
                let (sender, receiver) = crossbeam_channel::unbounded();
                runtime.send(events, sender).unwrap();
                assert_eq!(receiver.recv_timeout(Duration::from_secs(2)), Err(crossbeam_channel::RecvTimeoutError::Disconnected));
            };
            let totals = || {
                let response = runtime.request_worker_status().unwrap();
                let mut inserted = 0;
                let mut replay = 0;
                for _ in 0..workers {
                    let status = response.recv_timeout(Duration::from_secs(2)).unwrap();
                    let counts = status.counters.unwrap();
                    inserted += counts.inserted_applications;
                    replay += counts.replay_applications.unwrap();
                    assert!(!counts.saturated);
                }
                (inserted, replay)
            };
            send(vec![Fanout(10), Fanout(30)]);
            assert_eq!(totals(), (6, 0));
            assert_eq!(runtime.query_at(fixture::Time(30), 1..=3).unwrap().iter().map(|s| s.count).sum::<u64>(), 6);
            assert_eq!(totals(), (6, 0));
            runtime.advance_to(fixture::Time(30)).unwrap();
            runtime.wait_until_idle(Duration::from_secs(2)).unwrap();
            assert_eq!(totals(), (6, 6));
            send(vec![Fanout(20), Fanout(20)]);
            runtime.wait_until_idle(Duration::from_secs(2)).unwrap();
            assert_eq!(totals(), (9, 15));
            assert_eq!(runtime.query_at(fixture::Time(30), 1..=3).unwrap().iter().map(|s| s.count).sum::<u64>(), 9);
            assert_eq!(totals(), (9, 15));
            runtime.shutdown();
        }
    }
}
