use std::hint::{black_box};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, BatchSize};

use contime::{SnapshotHistory};

mod helpers;
use helpers::{BenchSnapshot, BenchEvent};

fn new_event(time: i64, value: i16) -> BenchEvent {
    let snapshot_id = 0;
    let event_id: u128 = time.try_into().unwrap();

    if value >= 0 {
        BenchEvent::Positive(snapshot_id, time, event_id, value.abs() as u16)
    }else{
        BenchEvent::Negative(snapshot_id, time, event_id, value.abs() as u16)
    }
}

fn benchmark_apply_event(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("apply_event");

    for size in [1_000] {
        group.bench_function(BenchmarkId::new("in_order", size), |bencher| {
            bencher.iter_batched_ref(
                || SnapshotHistory::<BenchSnapshot>::new(0),
                |history| {
                    for i in 0..size {
                        history.apply_event(new_event(i, i.try_into().unwrap()));    
                    }
                    
                    black_box(&history);
                },
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("out_of_order_best_case", size), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut history = SnapshotHistory::<BenchSnapshot>::new(0);
                    history.apply_event(new_event(size, 1));
                    history
                },
                |history| {
                    for i in 0..size {
                        history.apply_event(new_event(i, i.try_into().unwrap()));    
                    }
                    
                    black_box(&history);
                }, 
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("out_of_order_average_case", size), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut history = SnapshotHistory::<BenchSnapshot>::new(0);
                    history.apply_event(new_event(size, 1));
                    history
                },
                |history| {
                    for i in 0..size {
                        history.apply_event(new_event(i/2, i.try_into().unwrap()));    
                    }
                    
                    black_box(&history);
                }, 
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("out_of_order_worst_case", size), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut history = SnapshotHistory::<BenchSnapshot>::new(0);
                    history.apply_event(new_event(size, 1));
                    history
                },
                |history| {
                    for i in 0..size {
                        history.apply_event(new_event(black_box(0), i.try_into().unwrap()));    
                        
                    }

                    black_box(&history);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

use pprof::criterion::{Output, PProfProfiler};

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = benchmark_apply_event
}

criterion_main!(benches);