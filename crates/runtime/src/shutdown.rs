use std::thread::JoinHandle;

use crate::{Runtime, ShutdownReport, ThreadOutcome};

impl<I, RE, WE> Runtime<I, RE, WE> {
    /// Closes the apply path and joins every router and worker thread.
    pub fn shutdown(mut self) -> ShutdownReport<RE, WE> {
        drop(self.input);
        let routers = join_all(std::mem::take(&mut self.routers));
        let workers = join_all(std::mem::take(&mut self.workers));
        ShutdownReport { routers, workers }
    }
}

fn join_all<E>(handles: Vec<JoinHandle<Result<(), E>>>) -> Vec<ThreadOutcome<E>> {
    handles
        .into_iter()
        .map(|handle| match handle.join() {
            Ok(Ok(())) => ThreadOutcome::Completed,
            Ok(Err(error)) => ThreadOutcome::Failed(error),
            Err(_) => ThreadOutcome::Panicked,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crossbeam_channel::{Receiver, Sender};

    use crate::{Router, Runtime, ThreadOutcome, Worker};

    struct FailingRouter(usize);

    impl Router for FailingRouter {
        type Input = ();
        type WorkerInput = ();
        type Error = usize;

        fn run(self, input: Receiver<Self::Input>, _workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
            for _ in input {}
            Err(self.0)
        }
    }

    struct FailingWorker(usize);

    impl Worker for FailingWorker {
        type Input = ();
        type Error = usize;

        fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
            for _ in input {}
            Err(self.0 + 10)
        }
    }

    #[test]
    fn shutdown_collects_every_returned_error_without_returning_early() {
        let runtime = Runtime::start(vec![FailingRouter(0), FailingRouter(1)], vec![FailingWorker(0), FailingWorker(1)]).unwrap();

        let report = runtime.shutdown();

        assert_eq!(report.routers, vec![ThreadOutcome::Failed(0), ThreadOutcome::Failed(1)]);
        assert_eq!(report.workers, vec![ThreadOutcome::Failed(10), ThreadOutcome::Failed(11)]);
    }

    struct PanickingRouter;

    impl Router for PanickingRouter {
        type Input = ();
        type WorkerInput = ();
        type Error = ();

        fn run(self, input: Receiver<Self::Input>, _workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
            for _ in input {}
            panic!("router panic")
        }
    }

    struct PanickingWorker;

    impl Worker for PanickingWorker {
        type Input = ();
        type Error = ();

        fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
            for _ in input {}
            panic!("worker panic")
        }
    }

    #[test]
    fn shutdown_distinguishes_router_and_worker_panics() {
        let runtime = Runtime::start(vec![PanickingRouter], vec![PanickingWorker]).unwrap();

        let report = runtime.shutdown();

        assert_eq!(report.routers, vec![ThreadOutcome::Panicked]);
        assert_eq!(report.workers, vec![ThreadOutcome::Panicked]);
    }

    struct OrderedRouter {
        returned: Arc<AtomicUsize>,
    }

    impl Router for OrderedRouter {
        type Input = ();
        type WorkerInput = ();
        type Error = ();

        fn run(self, input: Receiver<Self::Input>, _workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
            for _ in input {}
            self.returned.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct OrderedWorker {
        routers_returned: Arc<AtomicUsize>,
        observed: Arc<AtomicUsize>,
    }

    impl Worker for OrderedWorker {
        type Input = ();
        type Error = ();

        fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
            for _ in input {}
            self.observed.store(self.routers_returned.load(Ordering::SeqCst), Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn worker_input_closes_after_every_router_returns() {
        let routers_returned = Arc::new(AtomicUsize::new(0));
        let worker_observed = Arc::new(AtomicUsize::new(0));
        let routers_for_factory = Arc::clone(&routers_returned);
        let routers_for_worker = Arc::clone(&routers_returned);
        let observed_for_worker = Arc::clone(&worker_observed);
        let runtime = Runtime::start(
            vec![
                OrderedRouter { returned: Arc::clone(&routers_for_factory) },
                OrderedRouter { returned: Arc::clone(&routers_for_factory) },
            ],
            vec![OrderedWorker { routers_returned: Arc::clone(&routers_for_worker), observed: Arc::clone(&observed_for_worker) }],
        )
        .unwrap();

        let report = runtime.shutdown();

        assert_eq!(report.routers, vec![ThreadOutcome::Completed, ThreadOutcome::Completed]);
        assert_eq!(report.workers, vec![ThreadOutcome::Completed]);
        assert_eq!(worker_observed.load(Ordering::SeqCst), 2);
    }
}
