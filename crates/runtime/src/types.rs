use std::fmt;
use std::io;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};

/// Process-local execution topology.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    pub router_count: usize,
    pub worker_count: usize,
}

/// Opaque router execution owned by one runtime thread.
pub trait Router: Send + 'static {
    type Input: Send + 'static;
    type WorkerInput: Send + 'static;
    type Error: Send + 'static;

    fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error>;
}

/// Opaque worker execution owned by one runtime thread.
pub trait Worker: Send + 'static {
    type Input: Send + 'static;
    type Error: Send + 'static;

    fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error>;
}

/// The thread whose startup failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeStage {
    Router { index: usize },
    Worker { index: usize },
}

/// Failure to construct a complete runtime topology.
#[derive(Debug)]
pub enum StartError {
    NoRouters,
    NoWorkers,
    ThreadSpawn { stage: RuntimeStage, source: io::Error },
}

impl fmt::Display for StartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRouters => formatter.write_str("the runtime requires at least one router"),
            Self::NoWorkers => formatter.write_str("the runtime requires at least one worker"),
            Self::ThreadSpawn { stage, source } => {
                write!(formatter, "failed to start {stage:?}: {source}")
            }
        }
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ThreadSpawn { source, .. } => Some(source),
            Self::NoRouters | Self::NoWorkers => None,
        }
    }
}

/// Final state of one runtime thread.
#[derive(Debug, Eq, PartialEq)]
pub enum ThreadOutcome<E> {
    Completed,
    Failed(E),
    Panicked,
}

/// Ordered outcomes collected after every runtime thread has been joined.
#[derive(Debug, Eq, PartialEq)]
pub struct ShutdownReport<RE, WE> {
    pub routers: Vec<ThreadOutcome<RE>>,
    pub workers: Vec<ThreadOutcome<WE>>,
}

/// A running apply topology.
pub struct Runtime<I, RE, WE> {
    pub(crate) input: Option<Sender<I>>,
    pub(crate) routers: Vec<JoinHandle<Result<(), RE>>>,
    pub(crate) workers: Vec<JoinHandle<Result<(), WE>>>,
}

#[cfg(test)]
mod tests {
    use super::{RuntimeConfig, ShutdownReport, ThreadOutcome};

    #[test]
    fn configuration_preserves_topology_counts() {
        let config = RuntimeConfig { router_count: 2, worker_count: 4 };
        assert_eq!(config.router_count, 2);
        assert_eq!(config.worker_count, 4);
    }

    #[test]
    fn shutdown_report_preserves_every_ordered_outcome() {
        let report: ShutdownReport<&str, &str> = ShutdownReport {
            routers: vec![ThreadOutcome::Completed, ThreadOutcome::Failed("router")],
            workers: vec![ThreadOutcome::Panicked, ThreadOutcome::Completed],
        };
        assert_eq!(report.routers.len(), 2);
        assert_eq!(report.workers.len(), 2);
        assert_eq!(report.routers[1], ThreadOutcome::Failed("router"));
    }
}
