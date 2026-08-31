use std::fmt;

use crossbeam_channel::Sender;

use crate::Runtime;

/// An apply input returned when the shared router queue is unavailable.
#[derive(Debug, Eq, PartialEq)]
pub struct RuntimeSendError<I> {
    pub input: I,
}

impl<I> fmt::Display for RuntimeSendError<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the runtime input queue is unavailable")
    }
}

impl<I> std::error::Error for RuntimeSendError<I> where I: fmt::Debug {}

impl<I, RE, WE> Runtime<I, RE, WE> {
    /// Borrows the shared input sender consumed by all routers.
    pub fn input(&self) -> &Sender<I> {
        &self.input
    }

    /// Sends one opaque apply input to the shared router queue.
    pub fn send(&self, input: I) -> Result<(), RuntimeSendError<I>> {
        self.input.send(input).map_err(|error| RuntimeSendError { input: error.0 })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crossbeam_channel::{Receiver, Sender};

    use crate::{Router, Runtime, Worker};

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
        let routers = (0..router_count).map(|index| RelayRouter { index, received_counts: Arc::clone(&received_counts) }).collect();
        Runtime::start(routers, vec![RecordingWorker { received }]).unwrap()
    }

    fn finish_test_runtime(runtime: Runtime<u64, (), ()>) {
        let report = runtime.shutdown();
        assert!(report.routers.iter().all(|outcome| matches!(outcome, crate::ThreadOutcome::Completed)));
        assert!(report.workers.iter().all(|outcome| matches!(outcome, crate::ThreadOutcome::Completed)));
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
    fn one_input_sender_feeds_every_router() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(vec![AtomicUsize::new(0), AtomicUsize::new(0)]);
        let runtime = test_runtime(2, received, counts);

        runtime.input().send(7).unwrap();
        finish_test_runtime(runtime);
    }

    #[test]
    fn send_returns_the_input_when_the_shared_queue_is_closed() {
        let (input, receiver) = crossbeam_channel::unbounded();
        drop(receiver);
        let runtime = Runtime::<u64, (), ()> { input, routers: Vec::new(), workers: Vec::new() };

        let error = runtime.send(42).unwrap_err();

        assert_eq!(error.input, 42);
    }

    #[test]
    fn shared_router_queue_delivers_every_input_exactly_once() {
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
}
