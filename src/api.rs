use flume::Receiver;

use crate::{Router, Event, SnapshotLanes, EventLanes, RouterError, Snapshot};
use crate::handle::{ApplyHandle, QueryHandle, QueryResult, AdvanceHandle, HandleError};
use crate::history::Reconciliation;

#[derive(Debug)]
pub enum ContimeError {
	MemoryFull,
	NotFound,
	WorkerDropped,
	RouterError(RouterError),
}

impl From<RouterError> for ContimeError {
	fn from(err: RouterError) -> Self {
		match err {
			RouterError::MemoryFull => ContimeError::MemoryFull,
			other => ContimeError::RouterError(other),
		}
	}
}

impl From<HandleError> for ContimeError {
	fn from(_: HandleError) -> Self {
		ContimeError::WorkerDropped
	}
}

pub struct Contime<SL: SnapshotLanes<Event = EL>, EL: EventLanes<SL>> {
	router: Router<SL, EL>,
}

impl<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL> + 'static> Contime<SL, EL> {
	pub fn new(
		worker_count: usize,
		memory_budget_bytes: u64
    ) -> Self {
		let router = Router::<SL, EL>::new(worker_count, memory_budget_bytes);

		Self { router }
	}

	// Sync methods (blocking)

	pub fn advance(&self, time: i64) -> Result<(), ContimeError> {
		Ok(self.router.advance(time)?)
	}

	pub fn apply_event<E: Event>(&self, event: E) -> Result<(), ContimeError> where EL: From<E>
	{
		Ok(self.router.apply_event(event.into())?)
	}

	pub fn apply_snapshot<S: Snapshot>(&self, snapshot: S) -> Result<(), ContimeError> where SL: From<S> {
		Ok(self.router.apply_snapshot(snapshot.into())?)
	}

	pub fn at<S>(&self, time: i64, snapshot_id: u128) -> Result<(S, Receiver<Reconciliation>), ContimeError> where S: Snapshot + From<SL> {
		match self.router.at(time, snapshot_id)? {
			QueryResult::Found(snapshot_lane, reconciliation_rx) => {
				Ok((snapshot_lane.into(), reconciliation_rx))
			}
			QueryResult::NotFound => Err(ContimeError::NotFound),
		}
	}

	// Async methods (handle-based)

	pub fn send_event<E: Event>(&self, event: E) -> Result<ApplyHandle, ContimeError> where EL: From<E> {
		Ok(self.router.send_event(event.into())?)
	}

	pub fn send_snapshot<S: Snapshot>(&self, snapshot: S) -> Result<ApplyHandle, ContimeError> where SL: From<S> {
		Ok(self.router.send_snapshot(snapshot.into())?)
	}

	pub fn query_at(&self, time: i64, snapshot_id: u128) -> Result<QueryHandle<SL>, ContimeError> {
		Ok(self.router.query_at(time, snapshot_id)?)
	}

	pub fn send_advance(&self, time: i64) -> Result<AdvanceHandle, ContimeError> {
		Ok(self.router.send_advance(time)?)
	}
}
