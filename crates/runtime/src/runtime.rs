use std::fmt;

use crossbeam_channel::Sender;

use crate::Runtime;

/// An apply input returned when its selected router is unavailable.
#[derive(Debug, Eq, PartialEq)]
pub struct RuntimeSendError<I> {
    pub router_index: usize,
    pub input: I,
}

impl<I> fmt::Display for RuntimeSendError<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "runtime router {} is unavailable", self.router_index)
    }
}

impl<I> std::error::Error for RuntimeSendError<I> where I: fmt::Debug {}

impl<I, RE, WE> Runtime<I, RE, WE> {
    /// Borrows every router input sender in stable router-index order.
    pub fn inputs(&self) -> &[Sender<I>] {
        &self.inputs
    }

    /// Borrows one router input sender by its stable index.
    pub fn input(&self, router_index: usize) -> Option<&Sender<I>> {
        self.inputs.get(router_index)
    }

    /// Sends one opaque apply input to the explicitly selected router.
    pub fn send(&self, router_index: usize, input: I) -> Result<(), RuntimeSendError<I>> {
        let Some(sender) = self.input(router_index) else {
            return Err(RuntimeSendError { router_index, input });
        };
        sender.send(input).map_err(|error| RuntimeSendError { router_index, input: error.0 })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

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
        runtime.inputs.clear();
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

        runtime.send(0, 42).unwrap();
        finish_test_runtime(runtime);

        assert_eq!(*received.lock().unwrap(), vec![42]);
    }

    #[test]
    fn input_returns_the_runtime_owned_sender() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(vec![AtomicUsize::new(0)]);
        let runtime = test_runtime(1, Arc::clone(&received), counts);

        runtime.input(0).unwrap().send(7).unwrap();
        finish_test_runtime(runtime);

        assert_eq!(*received.lock().unwrap(), vec![7]);
    }

    #[test]
    fn inputs_exposes_one_sender_per_router() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(vec![AtomicUsize::new(0), AtomicUsize::new(0)]);
        let runtime = test_runtime(2, received, counts);

        assert_eq!(runtime.inputs().len(), 2);
        finish_test_runtime(runtime);
    }

    #[test]
    fn send_returns_the_input_when_the_router_index_is_invalid() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(vec![AtomicUsize::new(0)]);
        let runtime = test_runtime(1, received, counts);

        let error = runtime.send(1, 42).unwrap_err();

        assert_eq!(error.router_index, 1);
        assert_eq!(error.input, 42);
        finish_test_runtime(runtime);
    }

    #[test]
    fn indexed_router_queues_deliver_every_input_to_the_selected_router() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(vec![AtomicUsize::new(0), AtomicUsize::new(0)]);
        let runtime = test_runtime(2, Arc::clone(&received), Arc::clone(&counts));

        for value in 0..1_000 {
            runtime.send(value as usize % 2, value).unwrap();
        }
        finish_test_runtime(runtime);

        let mut received = received.lock().unwrap();
        received.sort_unstable();
        assert_eq!(*received, (0..1_000).collect::<Vec<_>>());
        assert_eq!(counts[0].load(Ordering::Relaxed), 500);
        assert_eq!(counts[1].load(Ordering::Relaxed), 500);
    }
}
