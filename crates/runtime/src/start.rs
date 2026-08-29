use std::io;
use std::thread::JoinHandle;

use crossbeam_channel::unbounded;

use crate::{Router, Runtime, RuntimeConfig, RuntimeStage, StartError, Worker};

trait Deps {
    fn spawn<T, F>(&self, name: String, run: F) -> io::Result<JoinHandle<T>>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static;
}

struct DefaultDeps;

impl Deps for DefaultDeps {
    fn spawn<T, F>(&self, name: String, run: F) -> io::Result<JoinHandle<T>>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        std::thread::Builder::new().name(name).spawn(run)
    }
}

impl Runtime<(), (), ()> {
    /// Starts a complete router and worker topology.
    pub fn start<R, W, RF, WF>(
        config: RuntimeConfig,
        router_factory: RF,
        worker_factory: WF,
    ) -> Result<Runtime<R::Input, R::Error, W::Error>, StartError>
    where
        R: Router<WorkerInput = W::Input>,
        W: Worker,
        RF: FnMut(usize) -> R,
        WF: FnMut(usize) -> W,
    {
        start_with_deps(&DefaultDeps, config, router_factory, worker_factory)
    }
}

fn start_with_deps<D, R, W, RF, WF>(
    deps: &D,
    config: RuntimeConfig,
    mut router_factory: RF,
    mut worker_factory: WF,
) -> Result<Runtime<R::Input, R::Error, W::Error>, StartError>
where
    D: Deps,
    R: Router<WorkerInput = W::Input>,
    W: Worker,
    RF: FnMut(usize) -> R,
    WF: FnMut(usize) -> W,
{
    if config.router_count == 0 {
        return Err(StartError::NoRouters);
    }
    if config.worker_count == 0 {
        return Err(StartError::NoWorkers);
    }

    let (input_sender, input_receiver) = unbounded::<R::Input>();
    let mut worker_senders = Vec::with_capacity(config.worker_count);
    let mut worker_receivers = Vec::with_capacity(config.worker_count);
    for _ in 0..config.worker_count {
        let (sender, receiver) = unbounded::<W::Input>();
        worker_senders.push(sender);
        worker_receivers.push(Some(receiver));
    }

    let mut workers = Vec::with_capacity(config.worker_count);
    for index in 0..config.worker_count {
        let worker = worker_factory(index);
        let receiver = worker_receivers[index].take().expect("each worker receiver is consumed exactly once");
        match deps.spawn(format!("contime-worker-{index}"), move || worker.run(receiver)) {
            Ok(handle) => workers.push(handle),
            Err(source) => {
                drop(input_sender);
                drop(input_receiver);
                drop(worker_senders);
                drop(worker_receivers);
                join_ignoring_outcomes(workers);
                return Err(StartError::ThreadSpawn { stage: RuntimeStage::Worker { index }, source });
            }
        }
    }
    drop(worker_receivers);

    let mut routers = Vec::with_capacity(config.router_count);
    for index in 0..config.router_count {
        let router = router_factory(index);
        let router_input = input_receiver.clone();
        let router_workers = worker_senders.clone();
        match deps.spawn(format!("contime-router-{index}"), move || router.run(router_input, router_workers)) {
            Ok(handle) => routers.push(handle),
            Err(source) => {
                drop(input_sender);
                drop(input_receiver);
                drop(worker_senders);
                join_ignoring_outcomes(routers);
                join_ignoring_outcomes(workers);
                return Err(StartError::ThreadSpawn { stage: RuntimeStage::Router { index }, source });
            }
        }
    }

    drop(input_receiver);
    drop(worker_senders);
    Ok(Runtime { input: Some(input_sender), routers, workers })
}

fn join_ignoring_outcomes<E>(handles: Vec<JoinHandle<Result<(), E>>>) {
    for handle in handles {
        let _ = handle.join();
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use crossbeam_channel::{unbounded, Receiver, Sender};

    use super::{start_with_deps, Deps};
    use crate::{Router, Runtime, RuntimeConfig, StartError, Worker};

    struct TestRouter;

    impl Router for TestRouter {
        type Input = u64;
        type WorkerInput = u64;
        type Error = ();

        fn run(self, input: Receiver<Self::Input>, _workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
            for _ in input {}
            Ok(())
        }
    }

    struct TestWorker;

    impl Worker for TestWorker {
        type Input = u64;
        type Error = ();

        fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
            for _ in input {}
            Ok(())
        }
    }

    #[test]
    fn zero_router_count_is_rejected_before_factories_run() {
        let router_calls = AtomicUsize::new(0);
        let worker_calls = AtomicUsize::new(0);
        let result = Runtime::start(
            RuntimeConfig { router_count: 0, worker_count: 1 },
            |_| {
                router_calls.fetch_add(1, Ordering::SeqCst);
                TestRouter
            },
            |_| {
                worker_calls.fetch_add(1, Ordering::SeqCst);
                TestWorker
            },
        );

        assert!(matches!(result, Err(StartError::NoRouters)));
        assert_eq!(router_calls.load(Ordering::SeqCst), 0);
        assert_eq!(worker_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn zero_worker_count_is_rejected_before_factories_run() {
        let result = Runtime::start(
            RuntimeConfig { router_count: 1, worker_count: 0 },
            |_| TestRouter,
            |_| TestWorker,
        );

        assert!(matches!(result, Err(StartError::NoWorkers)));
    }

    #[test]
    fn startup_uses_stable_factory_indexes_and_thread_counts() {
        let router_indexes = Arc::new(Mutex::new(Vec::new()));
        let worker_indexes = Arc::new(Mutex::new(Vec::new()));
        let router_indexes_for_factory = Arc::clone(&router_indexes);
        let worker_indexes_for_factory = Arc::clone(&worker_indexes);

        let mut runtime = Runtime::start(
            RuntimeConfig { router_count: 2, worker_count: 4 },
            move |index| {
                router_indexes_for_factory.lock().unwrap().push(index);
                TestRouter
            },
            move |index| {
                worker_indexes_for_factory.lock().unwrap().push(index);
                TestWorker
            },
        )
        .unwrap();

        assert_eq!(runtime.routers.len(), 2);
        assert_eq!(runtime.workers.len(), 4);
        drop(runtime.input.take());
        for handle in runtime.routers.drain(..) {
            assert_eq!(handle.join().unwrap(), Ok(()));
        }
        for handle in runtime.workers.drain(..) {
            assert_eq!(handle.join().unwrap(), Ok(()));
        }
        assert_eq!(*router_indexes.lock().unwrap(), vec![0, 1]);
        assert_eq!(*worker_indexes.lock().unwrap(), vec![0, 1, 2, 3]);
    }

    struct StubDeps {
        calls: AtomicUsize,
        failing_call: usize,
    }

    impl StubDeps {
        fn failing_on(failing_call: usize) -> Self {
            Self { calls: AtomicUsize::new(0), failing_call }
        }
    }

    impl Deps for StubDeps {
        fn spawn<T, F>(&self, name: String, run: F) -> io::Result<JoinHandle<T>>
        where
            T: Send + 'static,
            F: FnOnce() -> T + Send + 'static,
        {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == self.failing_call {
                return Err(io::Error::other("stub spawn failure"));
            }
            std::thread::Builder::new().name(name).spawn(run)
        }
    }

    struct ExitRouter {
        exited: Sender<()>,
    }

    impl Router for ExitRouter {
        type Input = ();
        type WorkerInput = ();
        type Error = ();

        fn run(self, input: Receiver<Self::Input>, _workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error> {
            for _ in input {}
            self.exited.send(()).unwrap();
            Ok(())
        }
    }

    struct ExitWorker {
        exited: Sender<()>,
    }

    impl Worker for ExitWorker {
        type Input = ();
        type Error = ();

        fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error> {
            for _ in input {}
            self.exited.send(()).unwrap();
            Ok(())
        }
    }

    #[test]
    fn worker_spawn_failure_closes_and_joins_workers_already_started() {
        let (worker_exited, worker_exits) = unbounded();
        let (router_exited, _router_exits) = unbounded();

        let result = start_with_deps(
            &StubDeps::failing_on(2),
            RuntimeConfig { router_count: 1, worker_count: 3 },
            |_| ExitRouter { exited: router_exited.clone() },
            |_| ExitWorker { exited: worker_exited.clone() },
        );

        assert!(matches!(
            result,
            Err(StartError::ThreadSpawn { stage: crate::RuntimeStage::Worker { index: 2 }, .. })
        ));
        assert_eq!(worker_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert_eq!(worker_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
    }

    #[test]
    fn router_spawn_failure_closes_and_joins_the_entire_partial_topology() {
        let (worker_exited, worker_exits) = unbounded();
        let (router_exited, router_exits) = unbounded();

        let result = start_with_deps(
            &StubDeps::failing_on(3),
            RuntimeConfig { router_count: 2, worker_count: 2 },
            |_| ExitRouter { exited: router_exited.clone() },
            |_| ExitWorker { exited: worker_exited.clone() },
        );

        assert!(matches!(
            result,
            Err(StartError::ThreadSpawn { stage: crate::RuntimeStage::Router { index: 1 }, .. })
        ));
        assert_eq!(router_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert_eq!(worker_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert_eq!(worker_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
    }
}
