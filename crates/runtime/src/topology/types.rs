use std::fmt;
use std::io;
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};

/// Opaque router execution owned by one runtime thread.
pub trait Router: Send + 'static {
    /// Opaque message taken from the shared competing-consumer router queue.
    type Input: Send + 'static;
    /// Opaque message sent to private worker queues.
    type WorkerInput: Send + 'static;
    /// Process error retained in the shutdown report without automatic recovery.
    type Error: Send + 'static;

    /// Consumes this process on its runtime-owned thread.
    ///
    /// `workers` follows the supplied worker collection order. Implementations
    /// should drain queued input until disconnect and then release worker senders
    /// and return, so shutdown can join the topology. Other routers may still be
    /// dispatching after this router dequeues a message. Returning an error stops
    /// only this process and is recorded when joined.
    fn run(self, input: Receiver<Self::Input>, workers: Vec<Sender<Self::WorkerInput>>) -> Result<(), Self::Error>;
}

/// Opaque worker execution owned by one runtime thread.
pub trait Worker: Send + 'static {
    /// Opaque message delivered to this worker’s private queue.
    type Input: Send + 'static;
    /// Process error retained in the shutdown report without automatic recovery.
    type Error: Send + 'static;

    /// Consumes this process on its runtime-owned thread.
    ///
    /// Implementations should drain their queue until all router-owned senders
    /// disconnect and return. The runtime cannot interrupt a blocked implementation.
    /// Errors and panics are collected at shutdown; neither restarts the worker.
    fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error>;
}

/// The thread whose startup failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeStage {
    /// Admission-stage identity available to higher-level lifecycle reporting.
    /// The topology started by [`ThreadTopology::start`] spawns only routers and workers.
    Admission,
    /// Router thread in the supplied router collection.
    Router {
        /// Zero-based router index.
        index: usize,
    },
    /// Worker thread in the supplied worker collection.
    Worker {
        /// Zero-based worker index.
        index: usize,
    },
}

/// Failure to construct a complete runtime topology.
#[derive(Debug)]
pub enum StartError {
    /// No router instances were supplied; no threads were started.
    NoRouters,
    /// No worker instances were supplied; no threads were started.
    NoWorkers,
    /// A thread could not start; already started threads were closed and joined.
    ThreadSpawn {
        /// Component whose thread creation failed.
        stage: RuntimeStage,
        /// Operating-system thread creation error.
        source: io::Error,
    },
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
    /// The process returned `Ok(())` and its thread joined normally.
    Completed,
    /// The process returned its implementation-defined error.
    Failed(E),
    /// Joining observed a panic; the panic payload is discarded.
    Panicked,
}

/// Ordered outcomes collected after every runtime thread has been joined.
#[derive(Debug, Eq, PartialEq)]
pub struct ShutdownReport<RE, WE> {
    /// One outcome per router, in supplied collection order rather than finish order.
    pub routers: Vec<ThreadOutcome<RE>>,
    /// One outcome per worker, in supplied collection order rather than finish order.
    pub workers: Vec<ThreadOutcome<WE>>,
}

/// Owned router/worker thread topology with one shared input sender.
///
/// Call [`Self::shutdown`] to close and join it explicitly. Dropping the handle
/// closes its sender and detaches the join handles; it does not wait for threads.
/// External sender clones keep input connected until they are dropped.
pub struct ThreadTopology<I, RE, WE> {
    pub(crate) input: Sender<I>,
    pub(crate) routers: Vec<JoinHandle<Result<(), RE>>>,
    pub(crate) workers: Vec<JoinHandle<Result<(), WE>>>,
    pub(crate) queue_checks: Vec<Arc<dyn Fn() -> bool + Send + Sync>>,
}

#[cfg(test)]
mod tests {
    use super::{ShutdownReport, ThreadOutcome};

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
