#[derive(Debug)]
pub struct InputBatch<I, C> {
    pub inputs: Vec<I>,
    pub completion: C,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RoutedInput<I> {
    pub snapshot_id: u128,
    pub input: I,
}

#[derive(Debug)]
pub struct WorkerBatch<I, C> {
    pub inputs: Vec<RoutedInput<I>>,
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
