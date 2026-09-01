use std::time::{Duration, Instant};

use contime_core::{checkpoints, ConTime, ConTimeConfig, Input};
use contime_memory::ConservativeTrackedSize;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::unbounded;

struct BenchEvent {
    id: u128,
    time: u64,
    snapshot_id: u128,
}

impl ConservativeTrackedSize for BenchEvent {
    fn conservative_tracked_size(&self) -> usize {
        1_024
    }
}

impl Input for BenchEvent {
    type Time = u64;

    fn event_id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> u64 {
        self.time
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct BenchSnapshot {
    time: u64,
    count: u64,
}

impl ConservativeTrackedSize for BenchSnapshot {
    fn conservative_tracked_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl checkpoints::Snapshot for BenchSnapshot {
    type Time = u64;

    fn set_time(&mut self, time: u64) {
        self.time = time;
    }
}

impl checkpoints::ApplyEvents<BenchEvent> for BenchSnapshot {
    fn create(_snapshot_id: u128, _first_event: &BenchEvent) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: checkpoints::ApplyBatch<'_, u64, BenchEvent>) {
        self.count += batch.events.len() as u64;
    }
}

#[derive(Clone, Copy)]
enum Workload {
    Clean,
    Anchor,
    Dirty,
}

impl Workload {
    fn name(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Anchor => "anchor",
            Self::Dirty => "dirty",
        }
    }
}

fn config(router_count: usize, worker_count: usize, dirty: bool) -> ConTimeConfig<u64> {
    ConTimeConfig {
        router_count,
        worker_count,
        router_seed: 9,
        memory_limit: 1_000_000_000,
        memory_buffer: 1_000_000,
        history_retention: 10,
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_secs(60),
            replays_per_receive: if dirty { 0 } else { 1_000 },
            deadline_compaction_minimum: 2_048,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: checkpoints::CheckpointConfig { interval: 2 },
    }
}

fn events(workload: Workload) -> Vec<BenchEvent> {
    let times: &[u64] = match workload {
        Workload::Clean | Workload::Dirty => &[1],
        Workload::Anchor => &[1, 5, 10],
    };
    let mut id = 0_u128;
    let mut events = Vec::with_capacity(1_000 * times.len());
    for snapshot_id in 0..1_000_u128 {
        for time in times {
            id += 1;
            events.push(BenchEvent { id, time: *time, snapshot_id });
        }
    }
    events
}

fn measure_once(router_count: usize, worker_count: usize, workload: Workload) -> Duration {
    let dirty = matches!(workload, Workload::Dirty);
    let contime = ConTime::<BenchEvent, BenchSnapshot, ()>::start(config(router_count, worker_count, dirty), ()).unwrap();
    let mut pending = None;
    if dirty {
        let (completion, done) = unbounded();
        contime.send(events(workload), completion).unwrap();
        let mut ready = false;
        for _ in 0..10_000 {
            if contime.query_at(20, 0..1_000).unwrap().len() == 1_000 {
                ready = true;
                break;
            }
            std::thread::yield_now();
        }
        assert!(ready, "dirty benchmark histories did not reach every worker");
        pending = Some(done);
    } else {
        contime.apply(events(workload)).unwrap();
    }
    let before = contime.used_memory();
    let target = if matches!(workload, Workload::Anchor) { 18 } else { 20 };

    let started = Instant::now();
    contime.advance_to(target).unwrap();
    let elapsed = started.elapsed();

    assert!(contime.used_memory() < before);
    if let Some(done) = pending {
        assert_eq!(done.into_iter().count(), 0);
    }
    contime.shutdown();
    elapsed
}

fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("core/advance/1000_histories");
    group.throughput(Throughput::Elements(1_000));
    for (router_count, worker_count) in [(1, 1), (1, 4), (1, 10), (2, 10)] {
        for workload in [Workload::Clean, Workload::Anchor, Workload::Dirty] {
            group.bench_with_input(
                BenchmarkId::new(workload.name(), format!("{router_count}r_{worker_count}w")),
                &(router_count, worker_count, workload),
                |bencher, (router_count, worker_count, workload)| {
                    bencher.iter_custom(|iterations| {
                        (0..iterations)
                            .map(|_| measure_once(*router_count, *worker_count, *workload))
                            .fold(Duration::ZERO, |total, elapsed| total + elapsed)
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
