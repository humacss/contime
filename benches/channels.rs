use std::hint::black_box;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::thread::{self, JoinHandle};

use criterion::{criterion_group, criterion_main, Criterion};
use crossbeam_channel::{bounded as crossbeam_bounded, Sender as CrossbeamSender};
use flume::{bounded as flume_bounded, Sender as FlumeSender};

#[derive(Clone, Debug)]
struct SmallSnapshot {
    id: u128,
    time: i64,
    sum: i32,
}

mod flume_impl {
    use super::*;

    enum PingRequest {
        Ping { reply: FlumeSender<()> },
        Shutdown,
    }

    pub struct PingWorker {
        request_tx: FlumeSender<PingRequest>,
        thread: Option<JoinHandle<()>>,
    }

    impl PingWorker {
        pub fn new() -> Self {
            let (request_tx, request_rx) = flume_bounded::<PingRequest>(1024);
            let thread = thread::spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    match request {
                        PingRequest::Ping { reply } => {
                            let _ = reply.send(());
                        }
                        PingRequest::Shutdown => break,
                    }
                }
            });

            Self { request_tx, thread: Some(thread) }
        }

        pub fn ping(&self) {
            let (reply_tx, reply_rx) = flume_bounded::<()>(1);
            self.request_tx.send(PingRequest::Ping { reply: reply_tx }).expect("request should send");
            reply_rx.recv().expect("reply should arrive");
        }
    }

    impl Drop for PingWorker {
        fn drop(&mut self) {
            let _ = self.request_tx.send(PingRequest::Shutdown);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    enum QueryReply {
        NotFound,
        FoundSmall(SmallSnapshot),
    }

    enum QueryRequest {
        SnapshotAt { snapshot_id: u128, time: i64, reply: FlumeSender<QueryReply> },
        Shutdown,
    }

    pub struct QueryWorker {
        request_tx: FlumeSender<QueryRequest>,
        thread: Option<JoinHandle<()>>,
    }

    impl QueryWorker {
        pub fn new() -> Self {
            let (request_tx, request_rx) = flume_bounded::<QueryRequest>(1024);
            let thread = thread::spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    match request {
                        QueryRequest::SnapshotAt { snapshot_id, time, reply } => {
                            if snapshot_id == u128::MAX {
                                let _ = reply.send(QueryReply::NotFound);
                            } else if snapshot_id == 1 {
                                let _ = reply.send(QueryReply::FoundSmall(SmallSnapshot { id: snapshot_id, time, sum: 1 }));
                            } else {
                                let _ = reply.send(QueryReply::FoundSmall(SmallSnapshot { id: snapshot_id, time, sum: 1 }));
                            }
                        }
                        QueryRequest::Shutdown => break,
                    }
                }
            });

            Self { request_tx, thread: Some(thread) }
        }

        pub fn query_not_found(&self) {
            let (reply_tx, reply_rx) = flume_bounded::<QueryReply>(1);
            self.request_tx
                .send(QueryRequest::SnapshotAt { snapshot_id: u128::MAX, time: 1, reply: reply_tx })
                .expect("request should send");
            match reply_rx.recv().expect("reply should arrive") {
                QueryReply::NotFound => {}
                _ => panic!("expected not found"),
            }
        }

        pub fn query_found_small(&self) {
            let (reply_tx, reply_rx) = flume_bounded::<QueryReply>(1);
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
    }

    impl Drop for QueryWorker {
        fn drop(&mut self) {
            let _ = self.request_tx.send(QueryRequest::Shutdown);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    pub fn bounded_create() {
        let channels = flume_bounded::<()>(1);
        black_box(channels);
    }

    pub fn bounded_create_send_recv_same_thread() {
        let (tx, rx) = flume_bounded::<u64>(1);
        tx.send(1).expect("send should succeed");
        let value = rx.recv().expect("recv should succeed");
        black_box(value);
    }
}

mod crossbeam_impl {
    use super::*;

    enum PingRequest {
        Ping { reply: CrossbeamSender<()> },
        Shutdown,
    }

    pub struct PingWorker {
        request_tx: CrossbeamSender<PingRequest>,
        thread: Option<JoinHandle<()>>,
    }

    impl PingWorker {
        pub fn new() -> Self {
            let (request_tx, request_rx) = crossbeam_bounded::<PingRequest>(1024);
            let thread = thread::spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    match request {
                        PingRequest::Ping { reply } => {
                            let _ = reply.send(());
                        }
                        PingRequest::Shutdown => break,
                    }
                }
            });

            Self { request_tx, thread: Some(thread) }
        }

        pub fn ping(&self) {
            let (reply_tx, reply_rx) = crossbeam_bounded::<()>(1);
            self.request_tx.send(PingRequest::Ping { reply: reply_tx }).expect("request should send");
            reply_rx.recv().expect("reply should arrive");
        }
    }

    impl Drop for PingWorker {
        fn drop(&mut self) {
            let _ = self.request_tx.send(PingRequest::Shutdown);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    enum QueryReply {
        NotFound,
        FoundSmall(SmallSnapshot),
    }

    enum QueryRequest {
        SnapshotAt { snapshot_id: u128, time: i64, reply: CrossbeamSender<QueryReply> },
        Shutdown,
    }

    pub struct QueryWorker {
        request_tx: CrossbeamSender<QueryRequest>,
        thread: Option<JoinHandle<()>>,
    }

    impl QueryWorker {
        pub fn new() -> Self {
            let (request_tx, request_rx) = crossbeam_bounded::<QueryRequest>(1024);
            let thread = thread::spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    match request {
                        QueryRequest::SnapshotAt { snapshot_id, time, reply } => {
                            if snapshot_id == u128::MAX {
                                let _ = reply.send(QueryReply::NotFound);
                            } else if snapshot_id == 1 {
                                let _ = reply.send(QueryReply::FoundSmall(SmallSnapshot { id: snapshot_id, time, sum: 1 }));
                            } else {
                                let _ = reply.send(QueryReply::FoundSmall(SmallSnapshot { id: snapshot_id, time, sum: 1 }));
                            }
                        }
                        QueryRequest::Shutdown => break,
                    }
                }
            });

            Self { request_tx, thread: Some(thread) }
        }

        pub fn query_not_found(&self) {
            let (reply_tx, reply_rx) = crossbeam_bounded::<QueryReply>(1);
            self.request_tx
                .send(QueryRequest::SnapshotAt { snapshot_id: u128::MAX, time: 1, reply: reply_tx })
                .expect("request should send");
            match reply_rx.recv().expect("reply should arrive") {
                QueryReply::NotFound => {}
                _ => panic!("expected not found"),
            }
        }

        pub fn query_found_small(&self) {
            let (reply_tx, reply_rx) = crossbeam_bounded::<QueryReply>(1);
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
    }

    impl Drop for QueryWorker {
        fn drop(&mut self) {
            let _ = self.request_tx.send(QueryRequest::Shutdown);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    pub fn bounded_create() {
        let channels = crossbeam_bounded::<()>(1);
        black_box(channels);
    }

    pub fn bounded_create_send_recv_same_thread() {
        let (tx, rx) = crossbeam_bounded::<u64>(1);
        tx.send(1).expect("send should succeed");
        let value = rx.recv().expect("recv should succeed");
        black_box(value);
    }
}

mod std_sync_impl {
    use super::*;

    enum PingRequest {
        Ping { reply: SyncSender<()> },
        Shutdown,
    }

    pub struct PingWorker {
        request_tx: SyncSender<PingRequest>,
        thread: Option<JoinHandle<()>>,
    }

    impl PingWorker {
        pub fn new() -> Self {
            let (request_tx, request_rx) = sync_channel::<PingRequest>(1024);
            let thread = thread::spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    match request {
                        PingRequest::Ping { reply } => {
                            let _ = reply.send(());
                        }
                        PingRequest::Shutdown => break,
                    }
                }
            });

            Self { request_tx, thread: Some(thread) }
        }

        pub fn ping(&self) {
            let (reply_tx, reply_rx) = sync_channel::<()>(1);
            self.request_tx.send(PingRequest::Ping { reply: reply_tx }).expect("request should send");
            reply_rx.recv().expect("reply should arrive");
        }
    }

    impl Drop for PingWorker {
        fn drop(&mut self) {
            let _ = self.request_tx.send(PingRequest::Shutdown);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    enum QueryReply {
        NotFound,
        FoundSmall(SmallSnapshot),
    }

    enum QueryRequest {
        SnapshotAt { snapshot_id: u128, time: i64, reply: SyncSender<QueryReply> },
        Shutdown,
    }

    pub struct QueryWorker {
        request_tx: SyncSender<QueryRequest>,
        thread: Option<JoinHandle<()>>,
    }

    impl QueryWorker {
        pub fn new() -> Self {
            let (request_tx, request_rx) = sync_channel::<QueryRequest>(1024);
            let thread = thread::spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    match request {
                        QueryRequest::SnapshotAt { snapshot_id, time, reply } => {
                            if snapshot_id == u128::MAX {
                                let _ = reply.send(QueryReply::NotFound);
                            } else if snapshot_id == 1 {
                                let _ = reply.send(QueryReply::FoundSmall(SmallSnapshot { id: snapshot_id, time, sum: 1 }));
                            } else {
                                let _ = reply.send(QueryReply::FoundSmall(SmallSnapshot { id: snapshot_id, time, sum: 1 }));
                            }
                        }
                        QueryRequest::Shutdown => break,
                    }
                }
            });

            Self { request_tx, thread: Some(thread) }
        }

        pub fn query_not_found(&self) {
            let (reply_tx, reply_rx) = sync_channel::<QueryReply>(1);
            self.request_tx
                .send(QueryRequest::SnapshotAt { snapshot_id: u128::MAX, time: 1, reply: reply_tx })
                .expect("request should send");
            match reply_rx.recv().expect("reply should arrive") {
                QueryReply::NotFound => {}
                _ => panic!("expected not found"),
            }
        }

        pub fn query_found_small(&self) {
            let (reply_tx, reply_rx) = sync_channel::<QueryReply>(1);
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
    }

    impl Drop for QueryWorker {
        fn drop(&mut self) {
            let _ = self.request_tx.send(QueryRequest::Shutdown);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    pub fn bounded_create() {
        let channels = sync_channel::<()>(1);
        black_box(channels);
    }

    pub fn bounded_create_send_recv_same_thread() {
        let (tx, rx) = sync_channel::<u64>(1);
        tx.send(1).expect("send should succeed");
        let value = rx.recv().expect("recv should succeed");
        black_box(value);
    }
}

fn benchmark_channel_roundtrip(runner: &mut Criterion) {
    let mut group = runner.benchmark_group("channel_roundtrip");

    group.bench_function("flume/bounded_create", |bencher| {
        bencher.iter(flume_impl::bounded_create);
    });
    group.bench_function("flume/bounded_create_send_recv_same_thread", |bencher| {
        bencher.iter(flume_impl::bounded_create_send_recv_same_thread);
    });
    group.bench_function("flume/empty_reply_ping", |bencher| {
        let worker = flume_impl::PingWorker::new();
        bencher.iter(|| worker.ping());
    });
    group.bench_function("flume/query_not_found", |bencher| {
        let worker = flume_impl::QueryWorker::new();
        bencher.iter(|| worker.query_not_found());
    });
    group.bench_function("flume/query_found_small", |bencher| {
        let worker = flume_impl::QueryWorker::new();
        bencher.iter(|| worker.query_found_small());
    });
    group.bench_function("crossbeam/bounded_create", |bencher| {
        bencher.iter(crossbeam_impl::bounded_create);
    });
    group.bench_function("crossbeam/bounded_create_send_recv_same_thread", |bencher| {
        bencher.iter(crossbeam_impl::bounded_create_send_recv_same_thread);
    });
    group.bench_function("crossbeam/empty_reply_ping", |bencher| {
        let worker = crossbeam_impl::PingWorker::new();
        bencher.iter(|| worker.ping());
    });
    group.bench_function("crossbeam/query_not_found", |bencher| {
        let worker = crossbeam_impl::QueryWorker::new();
        bencher.iter(|| worker.query_not_found());
    });
    group.bench_function("crossbeam/query_found_small", |bencher| {
        let worker = crossbeam_impl::QueryWorker::new();
        bencher.iter(|| worker.query_found_small());
    });
    group.bench_function("std_sync/bounded_create", |bencher| {
        bencher.iter(std_sync_impl::bounded_create);
    });
    group.bench_function("std_sync/bounded_create_send_recv_same_thread", |bencher| {
        bencher.iter(std_sync_impl::bounded_create_send_recv_same_thread);
    });
    group.bench_function("std_sync/empty_reply_ping", |bencher| {
        let worker = std_sync_impl::PingWorker::new();
        bencher.iter(|| worker.ping());
    });
    group.bench_function("std_sync/query_not_found", |bencher| {
        let worker = std_sync_impl::QueryWorker::new();
        bencher.iter(|| worker.query_not_found());
    });
    group.bench_function("std_sync/query_found_small", |bencher| {
        let worker = std_sync_impl::QueryWorker::new();
        bencher.iter(|| worker.query_found_small());
    });
    group.finish();
}

criterion_group!(benches, benchmark_channel_roundtrip);
criterion_main!(benches);
