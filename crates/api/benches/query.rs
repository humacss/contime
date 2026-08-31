use std::hint::black_box;
use std::thread;

use contime_api::{query_at, query_events_between, EventQueryOutput, SnapshotQueryOutput};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use crossbeam_channel::{unbounded, Receiver, Sender};

enum Message {
    Snapshots { snapshot_ids: Vec<u128>, response: Sender<Vec<Box<u64>>> },
    Events { response: Sender<Vec<u64>> },
}

impl SnapshotQueryOutput<u64, u64> for Message {
    fn snapshot_query(_time: u64, snapshot_ids: Vec<u128>, response: Sender<Vec<Box<u64>>>) -> Self {
        Self::Snapshots { snapshot_ids, response }
    }
}

impl EventQueryOutput<u64, u64> for Message {
    fn event_query(_snapshot_id: u128, _from: u64, _to: u64, response: Sender<Vec<u64>>) -> Self {
        Self::Events { response }
    }
}

fn serve(receiver: Receiver<Message>, event_count: usize) {
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Snapshots { snapshot_ids, response } => {
                let results = snapshot_ids.into_iter().map(|snapshot_id| Box::new(snapshot_id as u64)).collect();
                let _ = response.send(results);
            }
            Message::Events { response } => {
                let _ = response.send((0..event_count as u64).collect());
            }
        }
    }
}

fn query_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("api/query");

    for count in [1_usize, 10, 100, 1_000] {
        let (output, receiver) = unbounded();
        let worker = thread::spawn(move || serve(receiver, count));
        let snapshot_ids = (0..count as u128).collect::<Vec<_>>();

        group.bench_with_input(BenchmarkId::new("snapshot_results", count), &count, |bencher, _| {
            bencher.iter(|| black_box(query_at(&output, black_box(42), snapshot_ids.iter().copied()).unwrap()));
        });
        group.bench_with_input(BenchmarkId::new("event_results", count), &count, |bencher, _| {
            bencher.iter(|| black_box(query_events_between(&output, 7, 0, 1_000).unwrap()));
        });

        drop(output);
        worker.join().unwrap();
    }

    group.finish();
}

criterion_group!(benches, query_benchmarks);
criterion_main!(benches);
