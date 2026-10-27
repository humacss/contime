#[path = "support/workers.rs"]
mod fixture;
use contime_monitor::{WorkerMonitor, WorkerMonitorError, WorkerStatus, WorkerStatusSource};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::Receiver;
use std::time::{Duration, Instant};
struct Ready(usize);
impl WorkerStatusSource<u64> for Ready {
    fn worker_count(&self) -> usize {
        self.0
    }
    fn request_worker_status(&self) -> Result<Receiver<WorkerStatus<u64>>, WorkerMonitorError> {
        let (sender, receiver) = crossbeam_channel::unbounded();
        for worker_id in 0..self.0 {
            sender
                .send(WorkerStatus {
                    worker_id,
                    history_stores: 128,
                    metadata_only: 0,
                    pending_histories: 4,
                    due_histories: 2,
                    processing_target: 10,
                    admission_horizon: 5,
                    pending_prune_passes: 0,
                    awaiting_resolution: None,
                    counters: None,
                })
                .unwrap();
        }
        Ok(receiver)
    }
}
fn benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("worker-monitor/ready-collection");
    for workers in [1, 4, 64] {
        let source = Ready(workers);
        let mut monitor = WorkerMonitor::attach(&source);
        group.bench_function(BenchmarkId::from_parameter(workers), |b| {
            b.iter(|| {
                monitor.begin(Duration::ZERO).unwrap();
                black_box(monitor.poll(Duration::ZERO).unwrap().unwrap());
            })
        });
    }
    group.finish();
    let source = Ready(4);
    let mut monitor = WorkerMonitor::attach(&source);
    c.bench_function("worker-monitor/begin-cancel", |b| {
        b.iter(|| {
            monitor.begin(Duration::ZERO).unwrap();
            monitor.cancel();
        })
    });
    monitor.begin(Duration::ZERO).unwrap();
    monitor.poll(Duration::ZERO).unwrap().unwrap();
    c.bench_function("worker-monitor/latest", |b| b.iter(|| black_box(monitor.latest())));
    c.bench_function("worker-monitor/history", |b| b.iter(|| black_box(monitor.history().count())));
    let mut group = c.benchmark_group("worker-monitor/runtime-request");
    group.sample_size(10);
    for workers in [1, 4] {
        for snapshots in [0, 128] {
            let runtime = fixture::start(workers);
            if snapshots > 0 {
                fixture::send(&runtime, 1, 4096, snapshots);
            }
            let mut monitor = WorkerMonitor::attach(&runtime);
            let origin = Instant::now();
            group.bench_function(BenchmarkId::new(format!("{workers}w"), snapshots), |b| {
                b.iter(|| {
                    monitor.begin(origin.elapsed()).unwrap();
                    loop {
                        if let Some(reading) = monitor.poll(origin.elapsed()).unwrap() {
                            black_box(reading);
                            break;
                        }
                        std::thread::yield_now();
                    }
                })
            });
            drop(monitor);
            runtime.shutdown();
        }
    }
    group.finish();
    let mut group = c.benchmark_group("worker-monitor/sustained-fixed-payload");
    group.sample_size(10).measurement_time(Duration::from_secs(5));
    const EVENTS: u64 = 4096;
    group.throughput(Throughput::Elements(EVENTS));
    let mut modes = [false, true];
    if std::env::var_os("CONTIME_MONITOR_REVERSE").is_some() {
        modes.reverse();
    }
    for workers in [1, 4] {
        for enabled in modes {
            let runtime = fixture::start(workers);
            let mut monitor = WorkerMonitor::attach(&runtime);
            let origin = Instant::now();
            let mut next_status = Duration::ZERO;
            let mut active = false;
            let mut next = 1;
            let mut collections = 0u64;
            group.bench_function(BenchmarkId::new(format!("{workers}w"), if enabled { "enabled" } else { "disabled" }), |b| {
                b.iter_custom(|iterations| {
                    let started = Instant::now();
                    for _ in 0..iterations {
                        // Identical advancement and source pacing in both variants.
                        runtime.advance_to(fixture::Time(next + EVENTS - 1)).unwrap();
                        fixture::send(&runtime, next, EVENTS, 128);
                        next += EVENTS;
                        if enabled {
                            let now = origin.elapsed();
                            if active && monitor.poll(now).unwrap().is_some() {
                                active = false;
                                collections += 1;
                            }
                            if !active && now >= next_status {
                                monitor.begin(now).unwrap();
                                active = true;
                                next_status = now + Duration::from_millis(50);
                            }
                        }
                    }
                    // Controlled benchmark only: equal final replay drain, never a monitor requirement.
                    runtime.wait_until_idle(Duration::from_secs(30)).unwrap();
                    started.elapsed()
                });
            });
            monitor.cancel();
            let states = runtime.query_at(fixture::Time(next - 1), 1..=128).unwrap();
            assert_eq!(states.iter().map(|s| s.count).sum::<u64>(), next - 1);
            eprintln!(
                "worker status sustained workers={workers} enabled={enabled} events={} completed_collections={collections}",
                next - 1
            );
            drop(monitor);
            runtime.shutdown();
        }
    }
    group.finish();
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
