//! Worker reports are independent observations, not an atomic global snapshot.
use contime_worker::WorkerStatus;
use crossbeam_channel::Receiver;
use std::{error::Error, fmt, time::Duration};

/// A read-only source of one report per worker, closed after the final response.
/// Implementations must enqueue without waiting for workers to finish.
pub trait WorkerStatusSource<T> {
    /// Number of workers, with IDs in `0..worker_count()`.
    fn worker_count(&self) -> usize;
    /// Starts collection; dropping the receiver cancels collection.
    fn request_worker_status(&self) -> Result<Receiver<WorkerStatus<T>>, WorkerMonitorError>;
}

/// A complete immutable collection. Workers may have reported at different times.
#[derive(Debug)]
pub struct WorkerReading<T> {
    /// Caller clock when the request was submitted.
    pub started_at: Duration,
    /// Caller clock when closure and all expected reports were observed.
    pub completed_at: Duration,
    /// One status per worker, sorted by worker ID.
    pub workers: Vec<WorkerStatus<T>>,
}

/// Request, response-integrity, or caller-clock errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerMonitorError {
    /// The source could not enqueue the request.
    Source(String),
    /// Another collection is still in flight.
    Busy,
    /// There is no collection to poll.
    NoActiveRequest,
    /// A source with no workers cannot be observed.
    EmptySource,
    /// The supplied clock is earlier than the last successful timed operation.
    ClockWentBackwards,
    /// A response identified an ID outside the source's worker range.
    UnknownWorker(usize),
    /// A worker responded more than once.
    DuplicateWorker(usize),
    /// The stream closed before all workers responded.
    Incomplete {
        /// Expected report count.
        expected: usize,
        /// Received report count.
        received: usize,
    },
}
impl fmt::Display for WorkerMonitorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => write!(f, "worker status request failed: {error}"),
            Self::Busy => f.write_str("worker status collection is already in flight"),
            Self::NoActiveRequest => f.write_str("no worker status collection is in flight"),
            Self::EmptySource => f.write_str("worker status source has no workers"),
            Self::ClockWentBackwards => f.write_str("worker monitor clock went backwards"),
            Self::UnknownWorker(id) => write!(f, "unknown worker status ID {id}"),
            Self::DuplicateWorker(id) => write!(f, "duplicate worker status ID {id}"),
            Self::Incomplete { expected, received } => write!(f, "worker status stream closed after {received} of {expected} reports"),
        }
    }
}
impl Error for WorkerMonitorError {}

#[cfg(feature = "runtime")]
impl<I, S, W> WorkerStatusSource<I::Time> for contime_runtime::ConTime<I, S, W>
where
    I: contime_runtime::ThreadInput,
{
    fn worker_count(&self) -> usize {
        self.worker_count()
    }
    fn request_worker_status(&self) -> Result<Receiver<WorkerStatus<I::Time>>, WorkerMonitorError> {
        self.request_worker_status().map_err(|error| WorkerMonitorError::Source(format!("{error:?}")))
    }
}
