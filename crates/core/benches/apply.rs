use std::hint::black_box;
use std::time::{Duration, Instant};

use contime_core::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
use contime_core::memory_tracking::ConservativeTrackedSize;
use contime_core::{ConTime, ConTimeConfig, Input, RejectionMessage, RejectionReason};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use crossbeam_channel::{unbounded, Receiver, Sender};

const EVENT_COUNT: usize = 1_000;
const BATCH_SIZES: [usize; 4] = [1, 10, 100, 1_000];
const SNAPSHOT_ID: u128 = 7;
const ROUTER_SEED: u64 = 9;
const TOPOLOGY_BATCH_COUNT: usize = 10;
const TOPOLOGIES: [(usize, usize); 6] = [(1, 1), (1, 2), (1, 4), (1, 8), (1, 10), (2, 10)];

struct BenchInput {
    id: u128,
    snapshot_id: u128,
    time: i64,
    value: usize,
    payload: [u8; 16],
}

const _: [(); 64] = [(); std::mem::size_of::<BenchInput>()];

impl ConservativeTrackedSize for BenchInput {
    fn conservative_tracked_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Input for BenchInput {
    type Time = i64;

    fn event_id(&self) -> u128 {
        self.id
    }

    fn time(&self) -> Self::Time {
        self.time
    }

    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.snapshot_id);
    }
}

#[derive(Clone, Default)]
struct BenchSnapshot {
    time: i64,
    value: usize,
}

impl ConservativeTrackedSize for BenchSnapshot {
    fn conservative_tracked_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Snapshot for BenchSnapshot {
    type Time = i64;

    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl ApplyEvents<BenchInput> for BenchSnapshot {
    fn create(_snapshot_id: u128, _first_event: &BenchInput) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, BenchInput>) {
        let added =
            batch.events.iter().fold(0_usize, |total, event| total.wrapping_add(event.value).wrapping_add(event.payload[0] as usize));
        self.value = black_box(self.value.wrapping_add(added));
    }
}

fn config(router_count: usize, worker_count: usize) -> ConTimeConfig<i64> {
    ConTimeConfig {
        router_count,
        worker_count,
        router_seed: ROUTER_SEED,
        memory_limit: 256 * 1024 * 1024,
        memory_buffer: 1024 * 1024,
        history_retention: 0,
        worker: contime_worker::WorkerConfig {
            maximum_dirty_age: Duration::from_micros(100),
            replays_per_receive: 1,
            deadline_compaction_minimum: 1_024,
            deadline_compaction_multiplier: 2,
        },
        checkpoints: CheckpointConfig { interval: 100 },
    }
}

fn input(id: usize) -> BenchInput {
    routed_input(id as u128, id as i64, SNAPSHOT_ID)
}

fn routed_input(id: u128, time: i64, snapshot_id: u128) -> BenchInput {
    BenchInput { id, snapshot_id, time, value: 1, payload: [0; 16] }
}

fn batches(batch_size: usize) -> Vec<Vec<BenchInput>> {
    (1..=EVENT_COUNT).step_by(batch_size).map(|first| (first..first + batch_size).map(input).collect()).collect()
}

struct PreparedWorkload {
    batches: Vec<(Vec<BenchInput>, Sender<RejectionMessage<RejectionReason>>)>,
    rejections: Receiver<RejectionMessage<RejectionReason>>,
}

fn prepare_workload(batches: Vec<Vec<BenchInput>>) -> PreparedWorkload {
    let (sender, rejections) = unbounded();
    let batches = batches.into_iter().map(|batch| (batch, sender.clone())).collect();
    drop(sender);
    PreparedWorkload { batches, rejections }
}

fn send_and_wait(contime: &ConTime<BenchInput, BenchSnapshot, ()>, workload: PreparedWorkload) -> usize {
    for (batch, sender) in workload.batches {
        contime.send(batch, sender).unwrap();
    }
    workload.rejections.into_iter().count()
}

fn measure(iterations: u64, batch_size: usize) -> Duration {
    let mut measured = Duration::ZERO;

    for _ in 0..iterations {
        let contime = ConTime::<BenchInput, BenchSnapshot, ()>::start(config(1, 1), ()).unwrap();
        assert_eq!(send_and_wait(&contime, prepare_workload(vec![vec![input(0)]])), 0);
        let workload = prepare_workload(batches(batch_size));

        let started = Instant::now();
        let rejection_count = send_and_wait(&contime, workload);
        measured += started.elapsed();

        assert_eq!(rejection_count, 0);
        black_box(contime.used_memory());
        let report = contime.shutdown();
        assert!(report.routers.iter().all(|outcome| *outcome == contime_runtime::ThreadOutcome::Completed));
        assert!(report.workers.iter().all(|outcome| *outcome == contime_runtime::ThreadOutcome::Completed));
    }

    measured
}

#[derive(Clone)]
struct ProbeInput(u128);

impl contime_router::RoutableInput for ProbeInput {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.0);
    }
}

fn snapshot_ids_for_workers(worker_count: usize, ids_per_worker: usize) -> Vec<Vec<u128>> {
    let candidate_count = (worker_count * ids_per_worker * 32).max(1_024);
    let (input_sender, input_receiver) = unbounded();
    input_sender
        .send(contime_router::InputBatch { inputs: (0..candidate_count as u128).map(ProbeInput).collect(), completion: () })
        .unwrap();
    drop(input_sender);

    let mut worker_senders = Vec::with_capacity(worker_count);
    let mut worker_receivers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let (sender, receiver) = unbounded::<contime_router::WorkerBatch<ProbeInput, ()>>();
        worker_senders.push(sender);
        worker_receivers.push(receiver);
    }

    contime_router::route(ROUTER_SEED, input_receiver, &worker_senders).unwrap();
    worker_receivers
        .into_iter()
        .map(|receiver| {
            let ids = receiver.recv().unwrap().inputs.into_iter().take(ids_per_worker).map(|route| route.snapshot_id).collect::<Vec<_>>();
            assert_eq!(ids.len(), ids_per_worker);
            ids
        })
        .collect()
}

fn topology_batches(snapshot_ids: &[Vec<u128>]) -> Vec<Vec<BenchInput>> {
    let events_per_batch_per_worker = EVENT_COUNT / TOPOLOGY_BATCH_COUNT;
    (0..TOPOLOGY_BATCH_COUNT)
        .map(|batch_index| {
            snapshot_ids
                .iter()
                .enumerate()
                .flat_map(|(worker_index, worker_snapshot_ids)| {
                    (0..events_per_batch_per_worker).map(move |offset| {
                        let sequence = batch_index * events_per_batch_per_worker + offset + 1;
                        let id = worker_index * EVENT_COUNT + sequence;
                        routed_input(id as u128, offset as i64 + 1, worker_snapshot_ids[batch_index])
                    })
                })
                .collect()
        })
        .collect()
}

fn topology_warmup(snapshot_ids: &[Vec<u128>]) -> Vec<BenchInput> {
    snapshot_ids.iter().flatten().enumerate().map(|(index, snapshot_id)| routed_input(u128::MAX - index as u128, 0, *snapshot_id)).collect()
}

fn measure_topology(iterations: u64, router_count: usize, worker_count: usize, snapshot_ids: &[Vec<u128>]) -> Duration {
    let mut measured = Duration::ZERO;

    for _ in 0..iterations {
        let contime = ConTime::<BenchInput, BenchSnapshot, ()>::start(config(router_count, worker_count), ()).unwrap();
        assert_eq!(send_and_wait(&contime, prepare_workload(vec![topology_warmup(snapshot_ids)])), 0);
        let workload = prepare_workload(topology_batches(snapshot_ids));

        let started = Instant::now();
        let rejection_count = send_and_wait(&contime, workload);
        measured += started.elapsed();
        assert_eq!(rejection_count, 0);

        black_box(contime.used_memory());
        let report = contime.shutdown();
        assert_eq!(report.routers.len(), router_count);
        assert_eq!(report.workers.len(), worker_count);
        assert!(report.routers.iter().all(|outcome| *outcome == contime_runtime::ThreadOutcome::Completed));
        assert!(report.workers.iter().all(|outcome| *outcome == contime_runtime::ThreadOutcome::Completed));
    }

    measured
}

fn benchmark_send_batches(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("core/send_end_to_end");
    group.throughput(Throughput::Elements(EVENT_COUNT as u64));

    for batch_size in BATCH_SIZES {
        let batch_count = EVENT_COUNT / batch_size;
        group.bench_with_input(
            BenchmarkId::new(format!("{batch_count}_batches"), format!("{batch_size}_events_each")),
            &batch_size,
            |bencher, batch_size| bencher.iter_custom(|iterations| measure(iterations, *batch_size)),
        );
    }

    group.finish();
}

fn benchmark_topologies(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("core/send_topology");

    for (router_count, worker_count) in TOPOLOGIES {
        let total_events = EVENT_COUNT * worker_count;
        let snapshot_ids = snapshot_ids_for_workers(worker_count, TOPOLOGY_BATCH_COUNT);
        group.throughput(Throughput::Elements(total_events as u64));
        group.bench_function(
            BenchmarkId::new(
                format!("{router_count}_routers_{worker_count}_workers"),
                format!("{TOPOLOGY_BATCH_COUNT}_batches_{EVENT_COUNT}_events_per_worker"),
            ),
            |bencher| bencher.iter_custom(|iterations| measure_topology(iterations, router_count, worker_count, &snapshot_ids)),
        );
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(20);
    targets = benchmark_send_batches, benchmark_topologies
}
criterion_main!(benches);
