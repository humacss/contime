use contime_monitor::{Monitor, Totals};
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use std::{hint::black_box, num::NonZeroUsize, time::Duration};

fn populated(capacity: usize) -> Monitor {
    let mut monitor = Monitor::new(NonZeroUsize::new(capacity).unwrap());
    monitor.sample(Duration::ZERO).unwrap();
    for tick in 1..=capacity {
        monitor.record_submission(Totals { events: 1, applications: 3 }).unwrap();
        monitor.sample(Duration::from_nanos(tick as u64)).unwrap();
    }
    monitor
}
fn benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("monitor/operations");
    group.sample_size(40).warm_up_time(Duration::from_millis(300)).measurement_time(Duration::from_secs(1));
    group.bench_function("default", |b| b.iter(|| black_box(Monitor::default())));
    group.bench_function("new", |b| b.iter(|| Monitor::new(black_box(NonZeroUsize::new(120).unwrap()))));
    for batch in [1, 64, 4096] {
        let mut monitor = Monitor::default();
        group.throughput(Throughput::Elements(batch));
        group.bench_with_input(BenchmarkId::new("record_submission", batch), &batch, |b, &batch| {
            b.iter(|| black_box(&mut monitor).record_submission(black_box(Totals { events: batch, applications: batch * 3 })).unwrap());
        });
    }
    // Remaining results report operations, not source events.
    group.throughput(Throughput::Elements(1));
    let mut overflow = Monitor::default();
    overflow.record_submission(Totals { events: u64::MAX, applications: u64::MAX }).unwrap();
    group.bench_function("submission_overflow", |b| {
        b.iter(|| black_box(&mut overflow).record_submission(black_box(Totals { events: 1, applications: 3 })))
    });
    for explicit in [false, true] {
        group.bench_function(if explicit { "start_query_through" } else { "start_query" }, |b| {
            b.iter_batched_ref(
                || {
                    let mut m = Monitor::default();
                    m.record_submission(Totals { events: 100, applications: 300 }).unwrap();
                    m
                },
                |m| {
                    if explicit {
                        m.start_query_through(black_box(Duration::ZERO), black_box(Totals { events: 50, applications: 150 })).unwrap()
                    } else {
                        m.start_query(black_box(Duration::ZERO)).unwrap()
                    }
                },
                BatchSize::NumIterations(64),
            )
        });
    }
    let mut active = Monitor::default();
    let token = active.start_query(Duration::from_secs(1)).unwrap();
    group.bench_function("query_already_active", |b| b.iter(|| black_box(&mut active).start_query(black_box(Duration::from_secs(1)))));
    let mut invalid = Monitor::default();
    group.bench_function("invalid_prefix", |b| {
        b.iter(|| black_box(&mut invalid).start_query_through(black_box(Duration::ZERO), black_box(Totals { events: 1, applications: 1 })))
    });
    group.bench_function("invalid_token", |b| {
        b.iter(|| black_box(&mut invalid).complete_query(black_box(token), black_box(Duration::from_secs(1))))
    });
    group.bench_function("clock_reversed", |b| b.iter(|| black_box(&active).snapshot(black_box(Duration::ZERO))));
    group.bench_function("first_sample_allocations", |b| {
        b.iter_batched_ref(Monitor::default, |m| m.sample(black_box(Duration::ZERO)).unwrap(), BatchSize::NumIterations(64))
    });
    for capacity in [1, 120, 4096] {
        let monitor = populated(capacity);
        group.bench_with_input(BenchmarkId::new("snapshot", capacity), &capacity, |b, _| {
            b.iter(|| black_box(&monitor).snapshot(black_box(Duration::from_secs(1))).unwrap())
        });
        group.bench_with_input(BenchmarkId::new("history", capacity), &capacity, |b, _| {
            b.iter(|| {
                let mut sum = 0u64;
                for sample in black_box(&monitor).history() {
                    sum = sum.wrapping_add(black_box(sample).submitted.applications);
                }
                black_box(sum)
            })
        });
        let mut monitor = populated(capacity);
        let mut tick = capacity as u64;
        group.bench_with_input(BenchmarkId::new("sample_history_rollover", capacity), &capacity, |b, _| {
            b.iter(|| {
                tick += 1;
                black_box(&mut monitor).sample(black_box(Duration::from_nanos(tick))).unwrap()
            })
        });
        let mut monitor = populated(capacity);
        let mut tick = capacity as u64;
        group.bench_with_input(BenchmarkId::new("sample_cohort_compaction", capacity), &capacity, |b, _| {
            b.iter(|| {
                tick += 1;
                monitor.record_submission(black_box(Totals { events: 1, applications: 3 })).unwrap();
                black_box(&mut monitor).sample(black_box(Duration::from_nanos(tick))).unwrap()
            })
        });
    }
    for cohorts in [0, 1, 120, 4096] {
        group.bench_with_input(BenchmarkId::new("complete_query", cohorts), &cohorts, |b, &n| {
            b.iter_batched_ref(
                || {
                    let mut m = if n == 0 { Monitor::default() } else { populated(n) };
                    let query = m.start_query(Duration::from_secs(1)).unwrap();
                    (m, query)
                },
                |(m, query)| m.complete_query(black_box(*query), black_box(Duration::from_secs(1))).unwrap(),
                if n >= 4096 { BatchSize::PerIteration } else { BatchSize::NumIterations(64) },
            )
        });
    }
    let monitor = active;
    macro_rules! getter {
        ($name:ident) => {
            group.bench_function(stringify!($name), |b| b.iter(|| black_box(&monitor).$name()));
        };
    }
    getter!(observed_at);
    getter!(submitted);
    getter!(confirmed);
    getter!(active_query);
    getter!(last_query);
    group.finish();
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
