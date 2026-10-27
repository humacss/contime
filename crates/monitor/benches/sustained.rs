#[allow(dead_code)]
#[path = "support/pipeline.rs"]
mod pipeline;
#[path = "support/sustained.rs"]
mod sustained;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput};
use pipeline::Mode;
use std::{
    hint::black_box,
    time::{Duration, Instant},
};
use sustained::{Config, Harness};
const EVENTS: u64 = 4096;
fn benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("monitor/sustained");
    group.sample_size(10).sampling_mode(SamplingMode::Flat).warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements(EVENTS));
    let mut modes = [Mode::Off, Mode::PerEvent];
    if std::env::var_os("CONTIME_MONITOR_REVERSE").is_some() {
        modes.reverse();
    }
    for workers in [1, 4] {
        for fanout in [1, 3] {
            for mode in modes {
                // One Runtime and monitor survive warmup and every measured sample.
                let mut harness = Harness::new(Config { workers, fanout, ..Config::default() }, mode);
                let origin = Instant::now();
                group.bench_function(BenchmarkId::new(format!("{}w-{}targets", workers, fanout), mode.label()), |b| {
                    b.iter_custom(|iterations| {
                        let started = Instant::now();
                        harness.run_events(iterations.checked_mul(EVENTS).unwrap(), &mut || origin.elapsed());
                        harness.settle(&mut || origin.elapsed());
                        black_box(harness.view());
                        started.elapsed()
                    });
                });
                let stats = harness.stats();
                // Criterion's --test smoke mode executes only one tiny iteration.
                // Full measurements must demonstrate periodic work actually ran.
                assert!(
                    stats.events == EVENTS || (stats.periodic_queries > 0 && stats.sample_ticks > 0),
                    "case must exercise periodic work"
                );
                assert_eq!(stats.events, stats.queried_events);
                assert!(stats.maximum_pending <= Config::default().maximum_pending);
                eprintln!("sustained {} workers={} fanout={} events={} periodic_queries={} capacity_queries={} final_queries={} sample_ticks={} peak_unqueried={} retained_monitor_samples={} pruned_horizon={}",mode.label(),workers,fanout,stats.events,stats.periodic_queries,stats.capacity_queries,stats.final_queries,stats.sample_ticks,stats.maximum_pending,stats.history_len,stats.pruned_horizon);
                harness.shutdown();
            }
        }
    }
    group.finish();
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
