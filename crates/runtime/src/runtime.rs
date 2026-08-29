use std::fmt;

use crossbeam_channel::Sender;

use crate::Runtime;

/// An apply input returned after every router receiver has closed.
#[derive(Debug, Eq, PartialEq)]
pub struct RuntimeSendError<I>(pub I);

impl<I> fmt::Display for RuntimeSendError<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the runtime input channel is closed")
    }
}

impl<I> std::error::Error for RuntimeSendError<I> where I: fmt::Debug {}

impl<I, RE, WE> Runtime<I, RE, WE> {
    /// Borrows the sender feeding the shared router queue.
    pub fn input(&self) -> &Sender<I> {
        self.input.as_ref().expect("a running runtime owns its input sender")
    }

    /// Sends one opaque apply input to the shared router queue.
    pub fn send(&self, input: I) -> Result<(), RuntimeSendError<I>> {
        self.input().send(input).map_err(|error| RuntimeSendError(error.0))
    }
}

#[cfg(test)]
mod tests {
    use std::hint::spin_loop;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use criterion::Criterion;
    use crossbeam_channel::{Receiver, Sender};

    use crate::{Router, Runtime, RuntimeConfig, Worker};

    struct RelayRouter {
        index: usize,
        received_counts: Arc<Vec<AtomicUsize>>,
    }

    impl Router for RelayRouter {
        type Input = u64;
        type WorkerInput = u64;
        type Error = ();

        fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
            for value in input {
                self.received_counts[self.index].fetch_add(1, Ordering::Relaxed);
                workers[0].send(value).unwrap();
            }
            Ok(())
        }
    }

    struct RecordingWorker {
        received: Arc<Mutex<Vec<u64>>>,
    }

    impl Worker for RecordingWorker {
        type Input = u64;
        type Error = ();

        fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
            self.received.lock().unwrap().extend(input);
            Ok(())
        }
    }

    fn test_runtime(router_count: usize, received: Arc<Mutex<Vec<u64>>>, received_counts: Arc<Vec<AtomicUsize>>) -> Runtime<u64, (), ()> {
        Runtime::start(
            RuntimeConfig { router_count, worker_count: 1 },
            move |index| RelayRouter { index, received_counts: Arc::clone(&received_counts) },
            move |_| RecordingWorker { received: Arc::clone(&received) },
        )
        .unwrap()
    }

    fn finish_test_runtime(mut runtime: Runtime<u64, (), ()>) {
        drop(runtime.input.take());
        for handle in runtime.routers.drain(..) {
            assert_eq!(handle.join().unwrap(), Ok(()));
        }
        for handle in runtime.workers.drain(..) {
            assert_eq!(handle.join().unwrap(), Ok(()));
        }
    }

    #[test]
    fn send_forwards_one_opaque_input_into_the_running_topology() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(vec![AtomicUsize::new(0)]);
        let runtime = test_runtime(1, Arc::clone(&received), counts);

        runtime.send(42).unwrap();
        finish_test_runtime(runtime);

        assert_eq!(*received.lock().unwrap(), vec![42]);
    }

    #[test]
    fn input_returns_the_runtime_owned_sender() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(vec![AtomicUsize::new(0)]);
        let runtime = test_runtime(1, Arc::clone(&received), counts);

        runtime.input().send(7).unwrap();
        finish_test_runtime(runtime);

        assert_eq!(*received.lock().unwrap(), vec![7]);
    }

    #[test]
    fn shared_router_queue_consumes_every_input_exactly_once() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(vec![AtomicUsize::new(0), AtomicUsize::new(0)]);
        let runtime = test_runtime(2, Arc::clone(&received), Arc::clone(&counts));

        for value in 0..1_000 {
            runtime.send(value).unwrap();
        }
        finish_test_runtime(runtime);

        let mut received = received.lock().unwrap();
        received.sort_unstable();
        assert_eq!(*received, (0..1_000).collect::<Vec<_>>());
        assert_eq!(counts.iter().map(|count| count.load(Ordering::Relaxed)).sum::<usize>(), 1_000);
    }

    #[test]
    fn warm_completion_waits_for_every_submitted_batch() {
        let runtime = WarmRuntime::start();

        runtime.submit(1_000, 1);
        runtime.wait_for(1_000);

        assert_eq!(runtime.completed(), 1_000);
        runtime.shutdown();
    }

    #[derive(Debug)]
    struct BenchmarkBatch {
        worker_hint: usize,
        logical_events: usize,
    }

    struct BenchmarkRouter;

    impl Router for BenchmarkRouter {
        type Input = BenchmarkBatch;
        type WorkerInput = BenchmarkBatch;
        type Error = ();

        fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
            for batch in input {
                let worker_index = batch.worker_hint % workers.len();
                workers[worker_index].send(batch).unwrap();
            }
            Ok(())
        }
    }

    struct BenchmarkWorker {
        completed_batches: Arc<AtomicUsize>,
        completed_events: Arc<AtomicUsize>,
    }

    impl Worker for BenchmarkWorker {
        type Input = BenchmarkBatch;
        type Error = ();

        fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
            for batch in input {
                self.completed_events.fetch_add(batch.logical_events, Ordering::Relaxed);
                self.completed_batches.fetch_add(1, Ordering::Release);
            }
            Ok(())
        }
    }

    struct WarmRuntime {
        runtime: Runtime<BenchmarkBatch, (), ()>,
        completed_batches: Arc<AtomicUsize>,
        completed_events: Arc<AtomicUsize>,
    }

    impl WarmRuntime {
        fn start() -> Self {
            let completed_batches = Arc::new(AtomicUsize::new(0));
            let completed_events = Arc::new(AtomicUsize::new(0));
            let batches_for_workers = Arc::clone(&completed_batches);
            let events_for_workers = Arc::clone(&completed_events);
            let runtime = Runtime::start(
                RuntimeConfig { router_count: 2, worker_count: 4 },
                |_| BenchmarkRouter,
                move |_| BenchmarkWorker {
                    completed_batches: Arc::clone(&batches_for_workers),
                    completed_events: Arc::clone(&events_for_workers),
                },
            )
            .unwrap();
            Self { runtime, completed_batches, completed_events }
        }

        fn submit(&self, batch_count: usize, logical_events: usize) {
            for worker_hint in 0..batch_count {
                self.runtime.send(BenchmarkBatch { worker_hint, logical_events }).unwrap();
            }
        }

        fn submit_direct(&self, batch_count: usize, logical_events: usize) {
            for worker_hint in 0..batch_count {
                self.runtime.input().send(BenchmarkBatch { worker_hint, logical_events }).unwrap();
            }
        }

        fn wait_for(&self, target: usize) {
            while self.completed_batches.load(Ordering::Acquire) < target {
                spin_loop();
            }
        }

        fn completed(&self) -> usize {
            self.completed_batches.load(Ordering::Acquire)
        }

        fn completed_events(&self) -> usize {
            self.completed_events.load(Ordering::Relaxed)
        }

        fn shutdown(self) {
            let report = self.runtime.shutdown();
            assert_eq!(report.routers, vec![crate::ThreadOutcome::Completed, crate::ThreadOutcome::Completed]);
            assert_eq!(
                report.workers,
                vec![
                    crate::ThreadOutcome::Completed,
                    crate::ThreadOutcome::Completed,
                    crate::ThreadOutcome::Completed,
                    crate::ThreadOutcome::Completed,
                ]
            );
        }
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_runtime() {
        const BATCHES_PER_ITERATION: usize = 1_000;
        const LOGICAL_EVENTS_PER_BATCH: usize = 1_000;

        let mut criterion = Criterion::default();
        let runtime = WarmRuntime::start();

        let mut runtime_target = runtime.completed();
        criterion.bench_function("runtime/throughput/runtime_send/1000_batches_1000_events_each", |bencher| {
            bencher.iter(|| {
                runtime_target += BATCHES_PER_ITERATION;
                runtime.submit(BATCHES_PER_ITERATION, LOGICAL_EVENTS_PER_BATCH);
                runtime.wait_for(runtime_target);
            });
        });

        let mut direct_target = runtime.completed();
        criterion.bench_function("runtime/throughput/direct_sender/1000_batches_1000_events_each", |bencher| {
            bencher.iter(|| {
                direct_target += BATCHES_PER_ITERATION;
                runtime.submit_direct(BATCHES_PER_ITERATION, LOGICAL_EVENTS_PER_BATCH);
                runtime.wait_for(direct_target);
            });
        });

        assert_eq!(runtime.completed_events(), runtime.completed() * LOGICAL_EVENTS_PER_BATCH);
        runtime.shutdown();
        criterion.final_summary();
    }
}
