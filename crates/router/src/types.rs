use std::sync::Arc;

#[derive(Debug)]
pub struct InputBatch<E, C> {
    pub inputs: Vec<Arc<E>>,
    pub completion: C,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RoutedInput<E> {
    pub snapshot_id: u128,
    pub input: Arc<E>,
}

#[derive(Debug)]
pub struct WorkerBatch<E, C> {
    pub inputs: Vec<RoutedInput<E>>,
    pub completion: C,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouterError {
    NoWorkers,
    WorkerUnavailable { worker_index: usize },
}

pub trait RoutableInput {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}
