use flume::Receiver;

use crate::SnapshotLanes;
use crate::history::Reconciliation;

#[derive(Debug)]
pub enum HandleError {
    WorkerDropped,
}

pub struct ApplyHandle {
    rxs: Vec<Receiver<()>>,
}

impl ApplyHandle {
    pub(crate) fn new(rxs: Vec<Receiver<()>>) -> Self {
        Self { rxs }
    }

    pub fn wait(self) -> Result<(), HandleError> {
        for rx in &self.rxs {
            rx.recv().map_err(|_| HandleError::WorkerDropped)?;
        }
        Ok(())
    }

    pub fn try_recv(&mut self) -> Result<Option<()>, HandleError> {
        let mut i = 0;
        while i < self.rxs.len() {
            match self.rxs[i].try_recv() {
                Ok(()) => { self.rxs.swap_remove(i); }
                Err(flume::TryRecvError::Empty) => { i += 1; }
                Err(flume::TryRecvError::Disconnected) => return Err(HandleError::WorkerDropped),
            }
        }
        if self.rxs.is_empty() { Ok(Some(())) } else { Ok(None) }
    }

    pub async fn recv_async(self) -> Result<(), HandleError> {
        for rx in &self.rxs {
            rx.recv_async().await.map_err(|_| HandleError::WorkerDropped)?;
        }
        Ok(())
    }
}

pub enum QueryResult<SL: SnapshotLanes> {
    Found(SL, Receiver<Reconciliation>),
    NotFound,
}

pub struct QueryHandle<SL: SnapshotLanes> {
    rx: Receiver<QueryResult<SL>>,
}

impl<SL: SnapshotLanes> QueryHandle<SL> {
    pub(crate) fn new(rx: Receiver<QueryResult<SL>>) -> Self {
        Self { rx }
    }

    pub fn wait(self) -> Result<QueryResult<SL>, HandleError> {
        self.rx.recv().map_err(|_| HandleError::WorkerDropped)
    }

    pub fn try_recv(&self) -> Result<Option<QueryResult<SL>>, HandleError> {
        match self.rx.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(flume::TryRecvError::Empty) => Ok(None),
            Err(flume::TryRecvError::Disconnected) => Err(HandleError::WorkerDropped),
        }
    }

    pub async fn recv_async(self) -> Result<QueryResult<SL>, HandleError> {
        self.rx.recv_async().await.map_err(|_| HandleError::WorkerDropped)
    }
}

pub struct AdvanceHandle {
    rxs: Vec<Receiver<()>>,
}

impl AdvanceHandle {
    pub(crate) fn new(rxs: Vec<Receiver<()>>) -> Self {
        Self { rxs }
    }

    pub fn wait(self) -> Result<(), HandleError> {
        for rx in &self.rxs {
            rx.recv().map_err(|_| HandleError::WorkerDropped)?;
        }
        Ok(())
    }

    pub fn try_recv(&mut self) -> Result<Option<()>, HandleError> {
        let mut i = 0;
        while i < self.rxs.len() {
            match self.rxs[i].try_recv() {
                Ok(()) => { self.rxs.swap_remove(i); }
                Err(flume::TryRecvError::Empty) => { i += 1; }
                Err(flume::TryRecvError::Disconnected) => return Err(HandleError::WorkerDropped),
            }
        }
        if self.rxs.is_empty() { Ok(Some(())) } else { Ok(None) }
    }

    pub async fn recv_async(self) -> Result<(), HandleError> {
        for rx in &self.rxs {
            rx.recv_async().await.map_err(|_| HandleError::WorkerDropped)?;
        }
        Ok(())
    }
}
