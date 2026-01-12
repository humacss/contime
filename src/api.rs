use crate::{Router, Event, SnapshotLanes, EventLanes, RouterError, Snapshot, SnapshotReceiver};

#[derive(Debug)]
pub enum ContimeError {
	RouterError(RouterError)
}

pub struct Contime<SL: SnapshotLanes<Event = EL>, EL: EventLanes<SL>> {
	router: Router<SL, EL>,
}

impl<SL: SnapshotLanes<Event = EL> + 'static, EL: EventLanes<SL> + 'static> Contime<SL, EL> {
	pub fn new() -> Self {
		let router = Router::<SL, EL>::new(1, 1_000);

		Self { router }
	}

	/*
	pub fn advance(&self, time: i64) {
		self.router.advance(time);
	}
	*/

	pub fn send<E: Event>(&self, event: E) -> Result<(), ContimeError> where EL: From<E> 
	{
		match self.router.send(event.into()) {
			Ok(()) => Ok(()),
			Err(router_error) => Err(ContimeError::RouterError(router_error))
		}
	}

	pub fn at<S: Snapshot>(&self, time: i64, snapshot_id: u128) -> Result<(S, SnapshotReceiver<SL, S>), ContimeError> where S: From<SL> {	
		let (snapshot_lane, snapshot_lane_rx) = self.router.at(time, snapshot_id)?;

		Ok((snapshot_lane.into(), SnapshotReceiver::new(snapshot_lane_rx)))
	}
}
