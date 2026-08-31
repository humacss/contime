use std::io;
use std::thread::JoinHandle;

use crossbeam_channel::unbounded;

use crate::{Router, Runtime, RuntimeStage, StartError, Worker};

trait Deps {
    fn spawn<T, F>(&self, name: String, run: F) -> io::Result<JoinHandle<T>>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static;
}

struct DefaultDeps;

type StartedRuntime<R, W> = Runtime<<R as Router>::Input, <R as Router>::Error, <W as Worker>::Error>;

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
    pub fn start<R, W>(routers: Vec<R>, workers: Vec<W>) -> Result<StartedRuntime<R, W>, StartError>
    where
        R: Router<WorkerInput = W::Input>,
        W: Worker,
    {
        start_with_deps(&DefaultDeps, routers, workers)
    }
}

fn start_with_deps<D, R, W>(deps: &D, routers: Vec<R>, workers: Vec<W>) -> Result<StartedRuntime<R, W>, StartError>
where
    D: Deps,
    R: Router<WorkerInput = W::Input>,
    W: Worker,
{
    if routers.is_empty() {
        return Err(StartError::NoRouters);
    }
    if workers.is_empty() {
        return Err(StartError::NoWorkers);
    }

    let (input_sender, input_receiver) = unbounded::<R::Input>();
    let mut worker_senders = Vec::with_capacity(workers.len());
    let mut worker_receivers = Vec::with_capacity(workers.len());
    for _ in 0..workers.len() {
        let (sender, receiver) = unbounded::<W::Input>();
        worker_senders.push(sender);
        worker_receivers.push(Some(receiver));
    }

    let mut worker_handles = Vec::with_capacity(workers.len());
    for (index, worker) in workers.into_iter().enumerate() {
        let receiver = worker_receivers[index].take().expect("each worker receiver is consumed exactly once");
        match deps.spawn(format!("contime-worker-{index}"), move || worker.run(receiver)) {
            Ok(handle) => worker_handles.push(handle),
            Err(source) => {
                drop(input_sender);
                drop(input_receiver);
                drop(worker_senders);
                drop(worker_receivers);
                join_ignoring_outcomes(worker_handles);
                return Err(StartError::ThreadSpawn { stage: RuntimeStage::Worker { index }, source });
            }
        }
    }
    drop(worker_receivers);

    let mut router_handles = Vec::with_capacity(routers.len());
    for (index, router) in routers.into_iter().enumerate() {
        let router_input = input_receiver.clone();
        let router_workers = worker_senders.clone();
        match deps.spawn(format!("contime-router-{index}"), move || router.run(router_input, router_workers)) {
            Ok(handle) => router_handles.push(handle),
            Err(source) => {
                drop(input_sender);
                drop(input_receiver);
                drop(worker_senders);
                join_ignoring_outcomes(router_handles);
                join_ignoring_outcomes(worker_handles);
                return Err(StartError::ThreadSpawn { stage: RuntimeStage::Router { index }, source });
            }
        }
    }

    drop(input_receiver);
    drop(worker_senders);
    Ok(Runtime { input: input_sender, routers: router_handles, workers: worker_handles })
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
    use std::thread::JoinHandle;
    use std::time::Duration;

    use crossbeam_channel::{unbounded, Receiver, Sender};

    use super::{start_with_deps, Deps};
    use crate::{Router, Runtime, StartError, Worker};

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
    fn start_accepts_preassembled_router_and_worker_instances() {
        let runtime = Runtime::start(vec![TestRouter, TestRouter], vec![TestWorker]).unwrap();

        let _input = runtime.input();
        let report = runtime.shutdown();
        assert_eq!(report.routers.len(), 2);
        assert_eq!(report.workers.len(), 1);
    }

    #[test]
    fn zero_router_count_is_rejected() {
        let result = Runtime::start(Vec::<TestRouter>::new(), vec![TestWorker]);

        assert!(matches!(result, Err(StartError::NoRouters)));
    }

    #[test]
    fn zero_worker_count_is_rejected() {
        let result = Runtime::start(vec![TestRouter], Vec::<TestWorker>::new());

        assert!(matches!(result, Err(StartError::NoWorkers)));
    }

    #[test]
    fn startup_preserves_the_supplied_process_counts() {
        let runtime = Runtime::start(vec![TestRouter, TestRouter], vec![TestWorker, TestWorker, TestWorker, TestWorker]).unwrap();

        let report = runtime.shutdown();

        assert_eq!(report.routers.len(), 2);
        assert_eq!(report.workers.len(), 4);
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
            vec![ExitRouter { exited: router_exited }],
            (0..3).map(|_| ExitWorker { exited: worker_exited.clone() }).collect(),
        );

        assert!(matches!(result, Err(StartError::ThreadSpawn { stage: crate::RuntimeStage::Worker { index: 2 }, .. })));
        assert_eq!(worker_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert_eq!(worker_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
    }

    #[test]
    fn router_spawn_failure_closes_and_joins_the_entire_partial_topology() {
        let (worker_exited, worker_exits) = unbounded();
        let (router_exited, router_exits) = unbounded();

        let result = start_with_deps(
            &StubDeps::failing_on(3),
            (0..2).map(|_| ExitRouter { exited: router_exited.clone() }).collect(),
            (0..2).map(|_| ExitWorker { exited: worker_exited.clone() }).collect(),
        );

        assert!(matches!(result, Err(StartError::ThreadSpawn { stage: crate::RuntimeStage::Router { index: 1 }, .. })));
        assert_eq!(router_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert_eq!(worker_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert_eq!(worker_exits.recv_timeout(Duration::from_secs(1)), Ok(()));
    }
}
