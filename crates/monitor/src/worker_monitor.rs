//! Nonblocking collection with a bounded immutable history.
use crate::contime_monitor::{WorkerMonitorError, WorkerReading, WorkerStatusSource};
use crate::contime_worker::WorkerStatus;
use crossbeam_channel::{Receiver, TryRecvError};
use std::{collections::VecDeque, num::NonZeroUsize, sync::Arc, time::Duration};
struct Pending<T> {
    started: Duration,
    receiver: Receiver<WorkerStatus<T>>,
    workers: Vec<Option<WorkerStatus<T>>>,
    received: usize,
}
/// Caller-driven worker monitoring. Borrows its source and never owns a runtime.
/// No threads, sleeps, application queries, or terminal dependencies are used.
/// A stalled collection remains pending until the caller cancels it.
pub struct WorkerMonitor<'a, T, S: WorkerStatusSource<T> + ?Sized> {
    source: &'a S,
    capacity: usize,
    clock: Duration,
    pending: Option<Pending<T>>,
    history: VecDeque<Arc<WorkerReading<T>>>,
}
impl<'a, T, S: WorkerStatusSource<T> + ?Sized> WorkerMonitor<'a, T, S> {
    /// Attaches with capacity for the most recent 120 complete readings.
    pub fn attach(source: &'a S) -> Self {
        Self { source, capacity: 120, clock: Duration::ZERO, pending: None, history: VecDeque::new() }
    }
    /// Attaches with an explicit nonzero history capacity.
    pub fn with_capacity(source: &'a S, capacity: NonZeroUsize) -> Self {
        Self { capacity: capacity.get(), ..Self::attach(source) }
    }
    /// Enqueues a status request. An existing collection must finish or be cancelled.
    pub fn begin(&mut self, at: Duration) -> Result<(), WorkerMonitorError> {
        self.check_clock(at)?;
        if self.pending.is_some() {
            return Err(WorkerMonitorError::Busy);
        }
        let count = self.source.worker_count();
        if count == 0 {
            return Err(WorkerMonitorError::EmptySource);
        }
        let receiver = self.source.request_worker_status()?;
        self.pending = Some(Pending { started: at, receiver, workers: std::iter::repeat_with(|| None).take(count).collect(), received: 0 });
        self.clock = at;
        Ok(())
    }
    /// Drains at most `worker_count + 1` immediately available messages.
    /// Returns a reading only after every worker replied and the stream closed.
    /// Malformed or incomplete streams are cancelled without replacing history.
    /// A backwards clock consumes no responses and leaves the collection intact.
    pub fn poll(&mut self, at: Duration) -> Result<Option<Arc<WorkerReading<T>>>, WorkerMonitorError> {
        self.check_clock(at)?;
        let mut pending = self.pending.take().ok_or(WorkerMonitorError::NoActiveRequest)?;
        for _ in 0..=pending.workers.len() {
            match pending.receiver.try_recv() {
                Ok(status) => {
                    let id = status.worker_id;
                    let slot = pending.workers.get_mut(id).ok_or(WorkerMonitorError::UnknownWorker(id))?;
                    if slot.is_some() {
                        return Err(WorkerMonitorError::DuplicateWorker(id));
                    }
                    *slot = Some(status);
                    pending.received += 1;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if pending.received != pending.workers.len() {
                        return Err(WorkerMonitorError::Incomplete { expected: pending.workers.len(), received: pending.received });
                    }
                    let reading = Arc::new(WorkerReading {
                        started_at: pending.started,
                        completed_at: at,
                        workers: pending.workers.into_iter().flatten().collect(),
                    });
                    if self.history.len() == self.capacity {
                        self.history.pop_front();
                    }
                    self.history.push_back(Arc::clone(&reading));
                    self.clock = at;
                    return Ok(Some(reading));
                }
            }
        }
        self.pending = Some(pending);
        self.clock = at;
        Ok(None)
    }
    /// Drops any response receiver. Previously completed readings are retained.
    pub fn cancel(&mut self) {
        self.pending = None;
    }
    /// Most recent complete reading, if any.
    pub fn latest(&self) -> Option<&Arc<WorkerReading<T>>> {
        self.history.back()
    }
    /// Bounded complete readings in oldest-to-newest order.
    pub fn history(&self) -> impl ExactSizeIterator<Item = &Arc<WorkerReading<T>>> {
        self.history.iter()
    }
    fn check_clock(&self, at: Duration) -> Result<(), WorkerMonitorError> {
        if at < self.clock {
            Err(WorkerMonitorError::ClockWentBackwards)
        } else {
            Ok(())
        }
    }
}
