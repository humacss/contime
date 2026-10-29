use crate::support::{self, Change, Time};
use contime::monitor::{ApplicationActivity, WorkerMonitor, WorkerReading, WorkerStatusSource};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn collect<S: WorkerStatusSource<Time>>(monitor: &mut WorkerMonitor<'_, Time, S>, clock: Instant) -> Arc<WorkerReading<Time>> {
    monitor.begin(clock.elapsed()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(reading) = monitor.poll(clock.elapsed()).unwrap() {
            return reading;
        }
        assert!(Instant::now() < deadline, "worker collection timed out");
        std::thread::yield_now();
    }
}
#[test]
fn collecting_real_workers_counts_fanout_and_excludes_snapshot_queries() {
    for workers in [1, 2] {
        let engine = support::runtime(workers);
        let clock = Instant::now();
        let mut monitor = WorkerMonitor::attach(&engine);
        let before = collect(&mut monitor, clock);
        engine
            .apply([
                Change { id: 1, at: Time(1), targets: vec![1, 2], delta: 3 },
                Change { id: 2, at: Time(2), targets: vec![2], delta: 5 },
            ])
            .unwrap();
        engine.advance_to(Time(2)).unwrap();
        engine.wait_until_idle(Duration::from_secs(5)).unwrap();
        let after = collect(&mut monitor, clock);
        let activity = ApplicationActivity::between(Some(&before), &after);
        assert_eq!(after.workers.len(), workers);
        assert_eq!(activity.inserted_applications, Some(3));
        assert_eq!(activity.replay_applications, Some(3));
        assert_eq!(engine.query_at(Time(2), [1, 2]).unwrap().iter().map(|s| s.value).sum::<i64>(), 11);
        let queried = collect(&mut monitor, clock);
        let unchanged = ApplicationActivity::between(Some(&after), &queried);
        assert_eq!(unchanged.inserted_applications, activity.inserted_applications);
        assert_eq!(unchanged.replay_applications, activity.replay_applications);
        assert_eq!(unchanged.inserted_per_second, Some(0.0));
        assert_eq!(unchanged.replay_per_second, Some(0.0));
        drop(monitor);
        engine.shutdown();
    }
}
