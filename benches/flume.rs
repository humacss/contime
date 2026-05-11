use std::hint::black_box;
use std::thread::{self, JoinHandle};

use criterion::{criterion_group, criterion_main, Criterion};
use flume::{bounded, Receiver, Sender};

#[derive(Clone, Debug)]
struct SmallSnapshot {
    id: u128,
    time: i64,
    sum: i32,
}

enum EmptyReplyRequest {
    Ping { reply: Sender<()> },
    Shutdown,
}

struct EmptyReplyWorker {
    request_tx: Sender<EmptyReplyRequest>,
    thread: Option<JoinHandle<()>>,
}

impl EmptyReplyWorker {
    fn new() -> Self {
        let (request_tx, request_rx) = bounded::<EmptyReplyRequest>(1024);
        let thread = thread::spawn(move || {
            while let Ok(request) = request_rx.recv() {
                match request {
                    EmptyReplyRequest::Ping { reply } => {
                        let _ = reply.send(());
                    }
                    EmptyReplyRequest::Shutdown => break,
                }
            }
        });

        Self { request_tx, thread: Some(thread) }
    }

    fn ping(&self) {
        let (reply_tx, reply_rx) = bounded::<()>(1);
        self.request_tx.send(EmptyReplyRequest::Ping { reply: reply_tx }).expect("request should send");
        reply_rx.recv().expect("reply should arrive");
    }
}

impl Drop for EmptyReplyWorker {
    fn drop(&mut self) {
        let _ = self.request_tx.send(EmptyReplyRequest::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum QueryReply {
    NotFound,
    FoundSmall(SmallSnapshot),
    FoundWithReceiver { snapshot: SmallSnapshot, reconciliation_rx: Receiver<()> },
}

enum QueryRequest {
    SnapshotAt { snapshot_id: u128, time: i64, reply: Sender<QueryReply> },
    Shutdown,
}

struct QueryWorker {
    request_tx: Sender<QueryRequest>,
    thread: Option<JoinHandle<()>>,
}

impl QueryWorker {
    fn new() -> Self {
        let (request_tx, request_rx) = bounded::<QueryRequest>(1024);
        let thread = thread::spawn(move || {
            while let Ok(request) = request_rx.recv() {
                match request {
                    QueryRequest::SnapshotAt { snapshot_id, time, reply } => {
                        if snapshot_id == u128::MAX {
                            let _ = reply.send(QueryReply::NotFound);
                        } else if snapshot_id == 1 {
                            let _ = reply.send(QueryReply::FoundSmall(SmallSnapshot { id: snapshot_id, time, sum: 1 }));
                        } else {
                            let (_reconciliation_tx, reconciliation_rx) = bounded::<()>(1000);
                            let _ = reply.send(QueryReply::FoundWithReceiver {
                                snapshot: SmallSnapshot { id: snapshot_id, time, sum: 1 },
                                reconciliation_rx,
                            });
                        }
                    }
                    QueryRequest::Shutdown => break,
                }
            }
        });

        Self { request_tx, thread: Some(thread) }
    }

    fn query_not_found(&self) {
        let (reply_tx, reply_rx) = bounded::<QueryReply>(1);
        self.request_tx.send(QueryRequest::SnapshotAt { snapshot_id: u128::MAX, time: 1, reply: reply_tx }).expect("request should send");
        match reply_rx.recv().expect("reply should arrive") {
            QueryReply::NotFound => {}
            _ => panic!("expected not found"),
        }
    }

    fn query_found_small(&self) {
        let (reply_tx, reply_rx) = bounded::<QueryReply>(1);
        self.request_tx.send(QueryRequest::SnapshotAt { snapshot_id: 1, time: 1, reply: reply_tx }).expect("request should send");
        match reply_rx.recv().expect("reply should arrive") {
            QueryReply::FoundSmall(snapshot) => {
                black_box(snapshot.id);
                black_box(snapshot.time);
                black_box(snapshot.sum);
            }
            _ => panic!("expected found small"),
        }
    }

    fn query_found_with_receiver(&self) {
        let (reply_tx, reply_rx) = bounded::<QueryReply>(1);
        self.request_tx.send(QueryRequest::SnapshotAt { snapshot_id: 2, time: 1, reply: reply_tx }).expect("request should send");
        match reply_rx.recv().expect("reply should arrive") {
            QueryReply::FoundWithReceiver { snapshot, reconciliation_rx } => {
                black_box(snapshot.id);
                black_box(snapshot.time);
                black_box(snapshot.sum);
                black_box(reconciliation_rx);
            }
            _ => panic!("expected found with receiver"),
        }
    }
}

impl Drop for QueryWorker {
    fn drop(&mut self) {
        let _ = self.request_tx.send(QueryRequest::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn benchmark_flume_roundtrip(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("flume_roundtrip");

    group.bench_function("bounded_create", |bencher| {
        bencher.iter(|| {
            let channels = bounded::<()>(1);
            black_box(channels);
        });
    });

    group.bench_function("bounded_create_send_recv_same_thread", |bencher| {
        bencher.iter(|| {
            let (tx, rx) = bounded::<u64>(1);
            tx.send(1).expect("send should succeed");
            let value = rx.recv().expect("recv should succeed");
            black_box(value);
        });
    });

    group.bench_function("empty_reply_ping", |bencher| {
        let worker = EmptyReplyWorker::new();
        bencher.iter(|| worker.ping());
    });

    group.bench_function("query_not_found", |bencher| {
        let worker = QueryWorker::new();
        bencher.iter(|| worker.query_not_found());
    });

    group.bench_function("query_found_small", |bencher| {
        let worker = QueryWorker::new();
        bencher.iter(|| worker.query_found_small());
    });

    group.bench_function("query_found_with_receiver", |bencher| {
        let worker = QueryWorker::new();
        bencher.iter(|| worker.query_found_with_receiver());
    });

    group.finish();
}

criterion_group!(benches, benchmark_flume_roundtrip);
criterion_main!(benches);
