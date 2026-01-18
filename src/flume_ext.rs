use std::marker::PhantomData;

use flume::Receiver;

use crate::{SnapshotLanes, Snapshot};

pub struct SnapshotReceiver<SL: SnapshotLanes, S: Snapshot> {
    inner: Receiver<SL>,
    _target: PhantomData<S>,
}

impl<SL: SnapshotLanes, S:Snapshot> SnapshotReceiver<SL, S>
{
    pub fn new(inner: Receiver<SL>) -> Self {
        Self { inner, _target: PhantomData }
    }
}

impl<SL: SnapshotLanes, S: Snapshot + std::convert::From<SL>> IntoIterator for SnapshotReceiver<SL, S>
{
	type Item = S;
	type IntoIter = SnapshotIter<SL, S>;

    fn into_iter(self) -> Self::IntoIter {
        Self::IntoIter { inner: self.inner, _target: PhantomData }
    }
}

pub struct SnapshotIter<SL, S> {
    inner: Receiver<SL>,
    _target: PhantomData<S>,
}

impl<SL: SnapshotLanes, S: Snapshot + std::convert::From<SL>> Iterator for SnapshotIter<SL, S>
{
	type Item = S;

    fn next(&mut self) -> Option<S> {
        let snapshot_lane = self.inner.recv().ok()?;
        
        Some(snapshot_lane.into())
    }
}
