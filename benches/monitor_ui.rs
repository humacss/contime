#[allow(dead_code, unused_imports)] // Benchmark selects a subset; UI tests run through example targets.
#[path = "../examples/support/ui/presentation.rs"]
mod support;
use contime_monitor::{WorkerReading, WorkerStatus};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ratatui::{backend::TestBackend, Terminal};
use std::{sync::Arc, time::Duration};
use support::workers::{Scroll, WorkerView};
fn reading(count: usize) -> Arc<WorkerReading<u64>> {
    Arc::new(WorkerReading {
        started_at: Duration::ZERO,
        completed_at: Duration::from_millis(1),
        workers: (0..count)
            .map(|worker_id| WorkerStatus {
                worker_id,
                history_stores: 1000,
                counters: cfg!(feature = "counters").then_some(contime_monitor::WorkerCounters {
                    inserted_applications: 1_000_000,
                    replay_applications: Some(2_000_000),
                    saturated: false,
                }),
                metadata_only: 1,
                pending_histories: 200,
                due_histories: 100,
                processing_target: 1000000,
                admission_horizon: 900000,
                pending_prune_passes: 1,
                awaiting_resolution: if worker_id % 2 == 0 { Some(7) } else { None },
            })
            .collect(),
    })
}
fn benchmarks(c: &mut Criterion) {
    let mut update = c.benchmark_group("terminal/update-reading");
    for count in [1, 4, 128, 4096] {
        let reading = reading(count);
        let mut view = WorkerView::default();
        update.bench_function(BenchmarkId::from_parameter(count), |b| {
            b.iter(|| {
                view.set_reading(black_box(Arc::clone(&reading)));
                black_box(&view);
            })
        });
    }
    update.finish();
    let mut group = c.benchmark_group("terminal/in-memory-frame");
    for count in [1, 4, 128, 4096] {
        for (width, height) in [(80, 24), (160, 40)] {
            let mut view = WorkerView::default();
            let mut baseline = reading(count);
            let initial = Arc::get_mut(&mut baseline).unwrap();
            initial.completed_at = Duration::ZERO;
            for worker in &mut initial.workers {
                if let Some(counters) = &mut worker.counters {
                    counters.inserted_applications = 0;
                    counters.replay_applications = Some(0);
                }
            }
            view.set_reading(baseline);
            view.set_reading(reading(count));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            group.bench_function(BenchmarkId::new(format!("{width}x{height}"), count), |b| {
                b.iter(|| {
                    // Scroll alternately so changed table cells also exercise buffer diffing.
                    view.scroll(Scroll::Down);
                    if view.offset() >= count.saturating_sub(height as usize) {
                        view.scroll(Scroll::Home);
                    }
                    let frame = terminal.draw(|frame| view.render(frame, frame.area(), black_box(Duration::from_secs(1)))).unwrap();
                    black_box(frame.buffer);
                })
            });
        }
    }
    group.finish();
    let mut view = WorkerView::<u64>::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    c.bench_function("terminal/waiting", |b| {
        b.iter(|| {
            black_box(terminal.draw(|frame| view.render(frame, frame.area(), Duration::ZERO)).unwrap().buffer);
        })
    });
    view.set_reading(reading(128));
    c.bench_function("terminal/scroll", |b| {
        b.iter(|| {
            view.scroll(black_box(Scroll::PageDown));
            black_box(view.offset());
            view.scroll(Scroll::Home);
        })
    });
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
