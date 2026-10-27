use contime_monitor::{ApplicationActivity, WorkerCounters, WorkerReading, WorkerStatus};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::time::Duration;
fn reading(count: usize, at: u64, applications: u64) -> WorkerReading<u64> {
    WorkerReading {
        started_at: Duration::from_secs(at),
        completed_at: Duration::from_secs(at),
        workers: (0..count)
            .map(|worker_id| WorkerStatus {
                worker_id,
                history_stores: 1000,
                metadata_only: 0,
                pending_histories: 0,
                due_histories: 0,
                processing_target: 0,
                admission_horizon: 0,
                pending_prune_passes: 0,
                awaiting_resolution: None,
                counters: Some(WorkerCounters {
                    inserted_applications: applications,
                    replay_applications: Some(applications * 2),
                    saturated: false,
                }),
            })
            .collect(),
    }
}
fn benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("monitor/application-activity");
    for count in [1, 4, 128, 4096] {
        let before = reading(count, 1, 1000);
        let after = reading(count, 2, 2000);
        group.bench_function(BenchmarkId::from_parameter(count), |b| {
            b.iter(|| black_box(ApplicationActivity::between(Some(black_box(&before)), black_box(&after))))
        });
    }
    group.finish();
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
