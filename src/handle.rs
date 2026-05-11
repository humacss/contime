use std::time::Duration;

use crossbeam_channel::Receiver as CrossbeamReceiver;
use flume::Receiver as FlumeReceiver;

use crate::history::{Reconciliation, SnapshotWakeCause};
use crate::SnapshotLanes;

/// Errors returned by handle-based APIs.
#[derive(Debug)]
pub enum HandleError {
    /// A worker stopped before it could send its reply.
    WorkerDropped,
}

/// Completion handle returned by `send_event` and `send_snapshot`.
pub struct ApplyHandle {
    rxs: Vec<CrossbeamReceiver<()>>,
}

impl ApplyHandle {
    pub(crate) fn new(rxs: Vec<CrossbeamReceiver<()>>) -> Self {
        Self { rxs }
    }

    /// Blocks until all workers for the request have completed.
    pub fn wait(self) -> Result<(), HandleError> {
        for rx in &self.rxs {
            rx.recv().map_err(|_| HandleError::WorkerDropped)?;
        }
        Ok(())
    }

    /// Non-blocking completion check.
    ///
    /// Returns `Ok(Some(()))` once every worker has completed.
    pub fn try_recv(&mut self) -> Result<Option<()>, HandleError> {
        let mut i = 0;
        while i < self.rxs.len() {
            match self.rxs[i].try_recv() {
                Ok(()) => {
                    self.rxs.swap_remove(i);
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    i += 1;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => return Err(HandleError::WorkerDropped),
            }
        }
        if self.rxs.is_empty() {
            Ok(Some(()))
        } else {
            Ok(None)
        }
    }

    /// Asynchronously waits for all workers for the request to complete.
    pub async fn recv_async(self) -> Result<(), HandleError> {
        self.wait()
    }
}

/// Result returned by a query.
pub enum QueryResult<SL: SnapshotLanes> {
    /// The requested snapshot was found.
    ///
    /// The receiver yields future reconciliation notifications for the same snapshot id.
    Found(SL, FlumeReceiver<Reconciliation>),
    /// No history exists for the requested snapshot id.
    NotFound,
}

/// Result returned by a snapshot-scoped event query.
pub enum QueryEventsResult<EL> {
    /// The requested snapshot history was found.
    ///
    /// Events are returned in deterministic `(time, event_id)` order.
    Found(Vec<EL>),
    /// No history exists for the requested snapshot id.
    NotFound,
}

/// Global snapshot wake emitted when one snapshot lane changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotWake<SL: SnapshotLanes> {
    /// The current materialized snapshot lane after applying the change.
    pub lane: SL,
    /// Concrete snapshot id used for routing and sharding.
    pub snapshot_id: u128,
    /// Earliest logical time affected by the change.
    pub from_time: i64,
    /// Latest logical time affected by the change.
    pub to_time: i64,
    /// Why the wake was emitted.
    pub cause: SnapshotWakeCause,
}

/// Unified wake subscription merged across every contime worker.
pub struct WakeSubscription<SL: SnapshotLanes> {
    rx: FlumeReceiver<SnapshotWake<SL>>,
    cleanup: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl<SL: SnapshotLanes> WakeSubscription<SL> {
    pub(crate) fn new(rx: FlumeReceiver<SnapshotWake<SL>>, cleanup: Option<Box<dyn FnOnce() + Send + 'static>>) -> Self {
        Self { rx, cleanup }
    }

    /// Blocks until the next global wake is available.
    pub fn recv(&self) -> Result<SnapshotWake<SL>, flume::RecvError> {
        self.rx.recv()
    }

    /// Returns the underlying receiver for internal event-driven integrations.
    pub fn receiver(&self) -> &FlumeReceiver<SnapshotWake<SL>> {
        &self.rx
    }

    /// Waits for one wake with a timeout.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<SnapshotWake<SL>, flume::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    /// Non-blocking wake check.
    pub fn try_recv(&self) -> Result<SnapshotWake<SL>, flume::TryRecvError> {
        self.rx.try_recv()
    }
}

impl<SL: SnapshotLanes> Drop for WakeSubscription<SL> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

/// Logical time advancement published by a `contime` runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeAdvance {
    /// New global current time after the advance completed.
    pub current_time: i64,
    /// Signed delta applied by the successful advance.
    pub delta: i64,
}

/// Subscription to successful logical time advancement.
pub struct TimeAdvanceSubscription {
    rx: FlumeReceiver<TimeAdvance>,
}

impl TimeAdvanceSubscription {
    pub(crate) fn new(rx: FlumeReceiver<TimeAdvance>) -> Self {
        Self { rx }
    }

    /// Blocks until the next time advance is available.
    pub fn recv(&self) -> Result<TimeAdvance, flume::RecvError> {
        self.rx.recv()
    }

    /// Returns the underlying receiver for event-driven integrations.
    pub fn receiver(&self) -> &FlumeReceiver<TimeAdvance> {
        &self.rx
    }

    /// Waits for one time advance with a timeout.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<TimeAdvance, flume::RecvTimeoutError> {
        self.rx.recv_timeout(timeout)
    }

    /// Non-blocking time advance check.
    pub fn try_recv(&self) -> Result<TimeAdvance, flume::TryRecvError> {
        self.rx.try_recv()
    }
}

/// Handle returned by [`crate::Contime::query_at`].
pub struct QueryHandle<SL: SnapshotLanes> {
    rx: CrossbeamReceiver<QueryResult<SL>>,
}

impl<SL: SnapshotLanes> QueryHandle<SL> {
    pub(crate) fn new(rx: CrossbeamReceiver<QueryResult<SL>>) -> Self {
        Self { rx }
    }

    /// Blocks until the query result is available.
    pub fn wait(self) -> Result<QueryResult<SL>, HandleError> {
        self.rx.recv().map_err(|_| HandleError::WorkerDropped)
    }

    /// Checks whether the query result is ready without blocking.
    pub fn try_recv(&self) -> Result<Option<QueryResult<SL>>, HandleError> {
        match self.rx.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(crossbeam_channel::TryRecvError::Empty) => Ok(None),
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(HandleError::WorkerDropped),
        }
    }

    /// Asynchronously waits for the query result.
    pub async fn recv_async(self) -> Result<QueryResult<SL>, HandleError> {
        self.wait()
    }
}

/// Handle returned by [`crate::Contime::query_events_between`].
pub struct EventQueryHandle<EL> {
    rx: CrossbeamReceiver<QueryEventsResult<EL>>,
}

impl<EL> EventQueryHandle<EL> {
    pub(crate) fn new(rx: CrossbeamReceiver<QueryEventsResult<EL>>) -> Self {
        Self { rx }
    }

    /// Blocks until the event query result is available.
    pub fn wait(self) -> Result<QueryEventsResult<EL>, HandleError> {
        self.rx.recv().map_err(|_| HandleError::WorkerDropped)
    }

    /// Checks whether the event query result is ready without blocking.
    pub fn try_recv(&self) -> Result<Option<QueryEventsResult<EL>>, HandleError> {
        match self.rx.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(crossbeam_channel::TryRecvError::Empty) => Ok(None),
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(HandleError::WorkerDropped),
        }
    }

    /// Asynchronously waits for the event query result.
    pub async fn recv_async(self) -> Result<QueryEventsResult<EL>, HandleError> {
        self.wait()
    }
}

/// Completion handle returned by [`crate::Contime::send_advance`].
pub struct AdvanceHandle {
    rxs: Vec<CrossbeamReceiver<()>>,
    on_complete: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl AdvanceHandle {
    pub(crate) fn new(rxs: Vec<CrossbeamReceiver<()>>) -> Self {
        Self { rxs, on_complete: None }
    }

    pub(crate) fn new_with_completion(rxs: Vec<CrossbeamReceiver<()>>, on_complete: Box<dyn FnOnce() + Send + 'static>) -> Self {
        Self { rxs, on_complete: Some(on_complete) }
    }

    fn complete(&mut self) {
        if let Some(on_complete) = self.on_complete.take() {
            on_complete();
        }
    }

    /// Blocks until all workers have processed the advance request.
    pub fn wait(mut self) -> Result<(), HandleError> {
        for rx in &self.rxs {
            rx.recv().map_err(|_| HandleError::WorkerDropped)?;
        }
        self.complete();
        Ok(())
    }

    /// Non-blocking completion check.
    ///
    /// Returns `Ok(Some(()))` once every worker has completed.
    pub fn try_recv(&mut self) -> Result<Option<()>, HandleError> {
        let mut i = 0;
        while i < self.rxs.len() {
            match self.rxs[i].try_recv() {
                Ok(()) => {
                    self.rxs.swap_remove(i);
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    i += 1;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => return Err(HandleError::WorkerDropped),
            }
        }
        if self.rxs.is_empty() {
            self.complete();
            Ok(Some(()))
        } else {
            Ok(None)
        }
    }

    /// Asynchronously waits for all workers to process the advance request.
    pub async fn recv_async(self) -> Result<(), HandleError> {
        self.wait()
    }
}
