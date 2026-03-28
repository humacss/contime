use flume::Receiver;

use crate::history::Reconciliation;
use crate::SnapshotLanes;

/// Errors returned by handle-based APIs.
#[derive(Debug)]
pub enum HandleError {
    /// A worker stopped before it could send its reply.
    WorkerDropped,
}

/// Completion handle returned by `send_event` and `send_snapshot`.
pub struct ApplyHandle {
    rxs: Vec<Receiver<()>>,
}

impl ApplyHandle {
    pub(crate) fn new(rxs: Vec<Receiver<()>>) -> Self {
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
                Err(flume::TryRecvError::Empty) => {
                    i += 1;
                }
                Err(flume::TryRecvError::Disconnected) => return Err(HandleError::WorkerDropped),
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
        for rx in &self.rxs {
            rx.recv_async().await.map_err(|_| HandleError::WorkerDropped)?;
        }
        Ok(())
    }
}

/// Result returned by a query.
pub enum QueryResult<SL: SnapshotLanes> {
    /// The requested snapshot was found.
    ///
    /// The receiver yields future reconciliation notifications for the same snapshot id.
    Found(SL, Receiver<Reconciliation>),
    /// No history exists for the requested snapshot id.
    NotFound,
}

/// Handle returned by [`crate::Contime::query_at`].
pub struct QueryHandle<SL: SnapshotLanes> {
    rx: Receiver<QueryResult<SL>>,
}

impl<SL: SnapshotLanes> QueryHandle<SL> {
    pub(crate) fn new(rx: Receiver<QueryResult<SL>>) -> Self {
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
            Err(flume::TryRecvError::Empty) => Ok(None),
            Err(flume::TryRecvError::Disconnected) => Err(HandleError::WorkerDropped),
        }
    }

    /// Asynchronously waits for the query result.
    pub async fn recv_async(self) -> Result<QueryResult<SL>, HandleError> {
        self.rx.recv_async().await.map_err(|_| HandleError::WorkerDropped)
    }
}

/// Completion handle returned by [`crate::Contime::send_advance`].
pub struct AdvanceHandle {
    rxs: Vec<Receiver<()>>,
}

impl AdvanceHandle {
    pub(crate) fn new(rxs: Vec<Receiver<()>>) -> Self {
        Self { rxs }
    }

    /// Blocks until all workers have processed the advance request.
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
                Err(flume::TryRecvError::Empty) => {
                    i += 1;
                }
                Err(flume::TryRecvError::Disconnected) => return Err(HandleError::WorkerDropped),
            }
        }
        if self.rxs.is_empty() {
            Ok(Some(()))
        } else {
            Ok(None)
        }
    }

    /// Asynchronously waits for all workers to process the advance request.
    pub async fn recv_async(self) -> Result<(), HandleError> {
        for rx in &self.rxs {
            rx.recv_async().await.map_err(|_| HandleError::WorkerDropped)?;
        }
        Ok(())
    }
}
