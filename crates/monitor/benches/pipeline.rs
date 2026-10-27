#[path = "support/pipeline.rs"]
mod pipeline;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use pipeline::{Harness, Mode};
use std::{hint::black_box, time::Duration};
const EVENTS: usize = 4096;
fn benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("monitor/pipeline");
    group.sample_size(20).warm_up_time(Duration::from_millis(500)).measurement_time(Duration::from_secs(1));
    group.throughput(Throughput::Elements(EVENTS as u64));
    // Reverse instrumentation order on a repeat run to reveal time/order drift.
    let mut modes = Mode::ALL;
    if std::env::var_os("CONTIME_MONITOR_REVERSE").is_some() {
        modes.reverse();
    }
    for workers in [1, 4] {
        for fanout in [1, 3] {
            for mode in modes {
                group.bench_with_input(BenchmarkId::new(format!("{}w-{}targets", workers, fanout), mode.label()), &mode, |b, &mode| {
                    b.iter_custom(|iterations| {
                        let mut elapsed = Duration::ZERO;
                        for _ in 0..iterations {
                            let mut harness = Harness::new(workers, 128, EVENTS, fanout);
                            let report = harness.run(black_box(mode));
                            elapsed += report.measured;
                            black_box((report.metrics, report.totals));
                            harness.shutdown();
                        }
                        elapsed
                    });
                });
            }
        }
    }
    group.finish();
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
