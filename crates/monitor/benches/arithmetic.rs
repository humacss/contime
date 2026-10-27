// Compile the actual private arithmetic source without exposing a benchmark API.
#[allow(dead_code, unused_imports)]
#[path = "../src/observation.rs"]
mod observation;
use criterion::{criterion_group, criterion_main, Criterion};
use observation::{Error, Rates, Totals};
use std::fmt::Write;
struct Sink(usize);
impl std::fmt::Write for Sink {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0 += text.len();
        Ok(())
    }
}
use std::{hint::black_box, time::Duration};
fn benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("monitor/arithmetic");
    group.sample_size(40).warm_up_time(Duration::from_millis(300)).measurement_time(Duration::from_secs(1));
    let current = Totals { events: 1_000, applications: 3_000 };
    let delta = Totals { events: 64, applications: 192 };
    group.bench_function("checked_add", |b| b.iter(|| black_box(current).checked_add(black_box(delta))));
    group.bench_function("checked_add_overflow", |b| {
        b.iter(|| black_box(current).checked_add(black_box(Totals { events: u64::MAX, applications: 0 })))
    });
    group.bench_function("since", |b| b.iter(|| black_box(current).since(black_box(delta))));
    group.bench_function("since_saturating", |b| b.iter(|| black_box(delta).since(black_box(current))));
    group.bench_function("rates", |b| b.iter(|| Rates::between(black_box(delta), black_box(Duration::from_millis(50)))));
    group.bench_function("rates_zero_interval", |b| b.iter(|| Rates::between(black_box(delta), black_box(Duration::ZERO))));
    group.bench_function("error_display", |b| {
        b.iter(|| {
            let mut sink = Sink(0);
            write!(&mut sink, "{}", black_box(Error::InvalidPrefix)).unwrap();
            black_box(sink.0)
        })
    });
    group.finish();
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
