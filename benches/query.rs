use std::hint::black_box;
use std::thread::{self, JoinHandle};

use contime::{QueryResult, SnapshotHistory};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use flume::{bounded, Receiver, Sender};

mod helpers;
use helpers::{BenchContime, BenchEvent, BenchSnapshot};

const MEMORY_BUDGET_BYTES: u64 = 8 * 1024 * 1024;

fn new_event(snapshot_id: u128, event_id: u128, time: i64) -> BenchEvent {
    BenchEvent::Positive(snapshot_id, time, event_id, 1)
}

fn seeded_history(event_count: usize) -> SnapshotHistory<BenchSnapshot> {
    let mut history = SnapshotHistory::<BenchSnapshot>::new(BenchSnapshot::default(), 0, 10_000).0;

    for index in 0..event_count {
        history.apply_event(new_event(0, index as u128, index as i64));
    }

    history
}

fn seeded_contime_one_lane(event_count: usize) -> BenchContime {
    let contime = BenchContime::with_history_horizon(1, MEMORY_BUDGET_BYTES, 10_000);

    for index in 0..event_count {
        contime.apply_event(new_event(0, index as u128, index as i64)).expect("seed event should apply");
    }

    contime
}

fn seeded_contime_many_lanes(lane_count: usize, events_per_lane: usize) -> BenchContime {
    let contime = BenchContime::with_history_horizon(4, MEMORY_BUDGET_BYTES, 10_000);

    for lane_id in 0..lane_count as u128 {
        for offset in 0..events_per_lane as u128 {
            let event_id = lane_id.saturating_mul(events_per_lane as u128).saturating_add(offset);
            contime.apply_event(new_event(lane_id, event_id, offset as i64)).expect("seed event should apply");
        }
    }

    contime
}

fn seeded_contime_snapshot_only() -> BenchContime {
    let contime = BenchContime::with_history_horizon(1, MEMORY_BUDGET_BYTES, 10_000);
    contime.apply_snapshot(BenchSnapshot { id: 0, time: 0, sum: 0 }).expect("seed snapshot should apply");
    contime
}

struct FreshReplyRoundtrip {
    request_tx: Sender<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl FreshReplyRoundtrip {
    fn new() -> Self {
        let (request_tx, request_rx) = bounded::<Sender<()>>(1024);
        let thread = thread::spawn(move || {
            while let Ok(reply_tx) = request_rx.recv() {
                let _ = reply_tx.send(());
            }
        });

        Self { request_tx, thread: Some(thread) }
    }

    fn roundtrip(&self) {
        let (reply_tx, reply_rx) = bounded::<()>(1);
        self.request_tx.send(reply_tx).expect("request should send");
        reply_rx.recv().expect("reply should arrive");
    }
}

impl Drop for FreshReplyRoundtrip {
    fn drop(&mut self) {
        let (replacement_tx, _replacement_rx) = bounded::<Sender<()>>(1);
        let original = std::mem::replace(&mut self.request_tx, replacement_tx);
        drop(original);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum SharedReplyRequest {
    Query(u64),
    Shutdown,
}

struct SharedReplyRoundtrip {
    request_tx: Sender<SharedReplyRequest>,
    reply_rx: Receiver<u64>,
    thread: Option<JoinHandle<()>>,
}

impl SharedReplyRoundtrip {
    fn new() -> Self {
        let (request_tx, request_rx) = bounded::<SharedReplyRequest>(1024);
        let (reply_tx, reply_rx) = bounded::<u64>(1024);
        let thread = thread::spawn(move || {
            while let Ok(request) = request_rx.recv() {
                match request {
                    SharedReplyRequest::Query(id) => {
                        let _ = reply_tx.send(id);
                    }
                    SharedReplyRequest::Shutdown => break,
                }
            }
        });

        Self { request_tx, reply_rx, thread: Some(thread) }
    }

    fn roundtrip(&self, id: u64) {
        self.request_tx.send(SharedReplyRequest::Query(id)).expect("request should send");
        let result = self.reply_rx.recv().expect("reply should arrive");
        black_box(result);
    }
}

impl Drop for SharedReplyRoundtrip {
    fn drop(&mut self) {
        let _ = self.request_tx.send(SharedReplyRequest::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn benchmark_snapshot_at(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("query_snapshot_at");

    for event_count in [32, 256] {
        group.bench_function(BenchmarkId::new("local_small_history", event_count), |bencher| {
            let mut history = seeded_history(event_count);
            bencher.iter(|| {
                let (snapshot, _reconciliation_rx) = history.snapshot_at((event_count / 2) as i64 + 1);
                black_box(snapshot);
            });
        });
    }

    group.finish();
}

fn benchmark_query_at_wait(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("query_at_wait");

    group.bench_function("flume_bounded_create", |bencher| {
        bencher.iter(|| {
            let channels = bounded::<()>(1);
            black_box(channels);
        });
    });

    group.bench_function("flume_bounded_create_send_recv", |bencher| {
        bencher.iter(|| {
            let (tx, rx) = bounded::<u64>(1);
            tx.send(1).expect("send should succeed");
            let value = rx.recv().expect("recv should succeed");
            black_box(value);
        });
    });

    group.bench_function("persistent_roundtrip_fresh_reply_channel", |bencher| {
        let worker = FreshReplyRoundtrip::new();
        bencher.iter(|| worker.roundtrip());
    });

    group.bench_function("persistent_roundtrip_shared_reply_channel", |bencher| {
        let worker = SharedReplyRoundtrip::new();
        let mut next_id = 0_u64;
        bencher.iter(|| {
            worker.roundtrip(next_id);
            next_id = next_id.wrapping_add(1);
        });
    });

    group.bench_function("not_found", |bencher| {
        let contime = BenchContime::with_history_horizon(1, MEMORY_BUDGET_BYTES, 10_000);
        bencher.iter(|| match contime.query_at(1, 999).unwrap().wait().unwrap() {
            QueryResult::Found(_, _) => panic!("expected not found"),
            QueryResult::NotFound => {}
        });
    });

    group.bench_function("snapshot_only_lane", |bencher| {
        let contime = seeded_contime_snapshot_only();
        bencher.iter(|| match contime.query_at(1, 0).unwrap().wait().unwrap() {
            QueryResult::Found(snapshot_lane, _reconciliation_rx) => {
                let snapshot: BenchSnapshot = snapshot_lane.into();
                black_box(snapshot);
            }
            QueryResult::NotFound => panic!("expected query result"),
        });
    });

    for event_count in [32, 256] {
        group.bench_function(BenchmarkId::new("single_lane", event_count), |bencher| {
            let contime = seeded_contime_one_lane(event_count);
            bencher.iter(|| match contime.query_at((event_count / 2) as i64 + 1, 0).unwrap().wait().unwrap() {
                QueryResult::Found(snapshot_lane, _reconciliation_rx) => {
                    let snapshot: BenchSnapshot = snapshot_lane.into();
                    black_box(snapshot);
                }
                QueryResult::NotFound => panic!("expected query result"),
            });
        });
    }

    for lane_count in [32, 256] {
        group.bench_function(BenchmarkId::new("many_independent_lanes", lane_count), |bencher| {
            let contime = seeded_contime_many_lanes(lane_count, 4);
            bencher.iter(|| {
                let mut sum = 0_i32;

                for lane_id in 0..lane_count as u128 {
                    match contime.query_at(4, lane_id).unwrap().wait().unwrap() {
                        QueryResult::Found(snapshot_lane, _reconciliation_rx) => {
                            let snapshot: BenchSnapshot = snapshot_lane.into();
                            sum += snapshot.sum;
                        }
                        QueryResult::NotFound => panic!("expected query result"),
                    }
                }

                black_box(sum);
            });
        });

        group.bench_function(BenchmarkId::new("many_independent_lanes_batch", lane_count), |bencher| {
            let contime = seeded_contime_many_lanes(lane_count, 4);
            let lane_ids = (0..lane_count as u128).collect::<Vec<_>>();
            bencher.iter(|| {
                let snapshots = contime.many_at(4, &lane_ids).unwrap();
                let sum = snapshots
                    .into_iter()
                    .map(|lane| {
                        let snapshot_lane = lane.expect("expected query result");
                        let snapshot: BenchSnapshot = snapshot_lane.into();
                        snapshot.sum
                    })
                    .sum::<i32>();
                black_box(sum);
            });
        });
    }

    group.finish();
}

use pprof::criterion::{Output, PProfProfiler};

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = benchmark_snapshot_at, benchmark_query_at_wait
}

criterion_main!(benches);
