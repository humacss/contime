use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use pprof::criterion::{Output, PProfProfiler};

use contime::{TestEvent, TestSnapshotContime};

const MEMORY_BUDGET_BYTES: u64 = 512 * 1024 * 1024;

fn event(event_id: u128, time: i64, snapshot_id: u128) -> TestEvent {
    TestEvent::Positive(snapshot_id, time, event_id, 1)
}

fn benchmark_schedule(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("schedule_event");

    group.bench_function("one_event", |bencher| {
        let contime = TestSnapshotContime::new(1, MEMORY_BUDGET_BYTES);
        let mut next_id = 1_u128;

        bencher.iter(|| {
            next_id = next_id.wrapping_add(1);
            contime
                .send_scheduled_event(event(next_id, 1_000_000, next_id))
                .expect("schedule should enqueue")
                .wait()
                .expect("schedule should complete");
        });
    });

    group.bench_function("cancel_one_event", |bencher| {
        let contime = TestSnapshotContime::new(1, MEMORY_BUDGET_BYTES);
        let mut next_id = 1_u128;

        bencher.iter(|| {
            next_id = next_id.wrapping_add(1);
            contime.schedule_event(event(next_id, 1_000_000, next_id)).expect("schedule should complete");
            contime.cancel_scheduled_event(next_id, 1_000_000).expect("cancel should enqueue").wait().expect("cancel should complete");
        });
    });

    group.finish();
}

fn benchmark_advance(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("scheduled_advance");

    group.bench_function("no_due_events", |bencher| {
        let contime = TestSnapshotContime::new(1, MEMORY_BUDGET_BYTES);
        let mut next_time = 1_i64;

        bencher.iter(|| {
            next_time += 1;
            contime.send_advance_to(next_time).expect("advance should enqueue").wait().expect("advance should complete");
        });
    });

    for count in [1_usize, 100, 10_000] {
        group.bench_with_input(BenchmarkId::new("release_due_events", count), &count, |bencher, &count| {
            bencher.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;

                for _ in 0..iters {
                    let contime = TestSnapshotContime::new(1, MEMORY_BUDGET_BYTES);
                    for i in 0..count {
                        contime.schedule_event(event(i as u128, 5, i as u128)).expect("schedule should complete");
                    }
                    let start = Instant::now();
                    contime.send_advance_to(5).expect("advance should enqueue").wait().expect("advance should complete");
                    elapsed += start.elapsed();
                }

                elapsed
            });
        });
    }

    group.finish();
}

fn benchmark_apply_latency(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("scheduled_apply_latency");

    group.bench_function("scheduled_event_apply", |bencher| {
        bencher.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;

            for _ in 0..iters {
                let contime = TestSnapshotContime::new(1, MEMORY_BUDGET_BYTES);
                contime.schedule_event(event(1, 5, 1)).expect("schedule should complete");
                let start = Instant::now();
                contime.send_advance_to(5).expect("advance should enqueue").wait().expect("advance should complete");
                elapsed += start.elapsed();
            }

            elapsed
        });
    });

    group.bench_function("immediate_event_apply", |bencher| {
        bencher.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;

            for _ in 0..iters {
                let contime = TestSnapshotContime::new(1, MEMORY_BUDGET_BYTES);
                let start = Instant::now();
                contime.send_event(event(1, 5, 1)).expect("event should enqueue").wait().expect("event should apply");
                elapsed += start.elapsed();
            }

            elapsed
        });
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = benchmark_schedule, benchmark_advance, benchmark_apply_latency
}

criterion_main!(benches);
