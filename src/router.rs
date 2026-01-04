use crate::{Worker, Snapshot, WorkerInbound, WorkerOutbound};

pub struct Router<S>
where
    S: Snapshot,
{
    worker: Worker<S>, // will store multiple workers later
}


impl<S> Router<S>
where
    S: Snapshot + 'static,
{
	// accept worker count later
    pub fn new() -> Self {
        let worker = Worker::new();
       
        Self { worker }
    }

    pub fn get_tx(&self) -> flume::Sender<WorkerInbound<S>> {
    	// will proxy this later and route messages
        self.worker.get_tx()
    }

    pub fn get_rx(&self) -> flume::Receiver<WorkerOutbound<S>> {
    	// will proxy this later and route messages
        self.worker.get_rx()
    }
}
