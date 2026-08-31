#[derive(Debug)]
pub struct InputBatch<I, C> {
    pub inputs: Vec<I>,
    pub completion: C,
}

/// A caller-selected batch consumed by the router.
pub trait RouteInputBatch {
    type Input;
    type Completion;

    fn into_parts(self) -> (Vec<Self::Input>, Self::Completion);
}

impl<I, C> RouteInputBatch for InputBatch<I, C> {
    type Input = I;
    type Completion = C;

    fn into_parts(self) -> (Vec<Self::Input>, Self::Completion) {
        (self.inputs, self.completion)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RoutedInput<I> {
    pub snapshot_id: u128,
    pub input: I,
}

/// Constructs one caller-selected route emitted by the router.
pub trait RouteOutput<I>: Sized {
    fn create(snapshot_id: u128, input: I) -> Self;
}

impl<I> RouteOutput<I> for RoutedInput<I> {
    fn create(snapshot_id: u128, input: I) -> Self {
        Self { snapshot_id, input }
    }
}

#[derive(Debug)]
pub struct WorkerBatch<I, C> {
    pub inputs: Vec<RoutedInput<I>>,
    pub completion: C,
}

/// Constructs one caller-selected worker message emitted by the router.
pub trait WorkerOutput<I, C>: Sized {
    type Route: RouteOutput<I>;

    fn create(inputs: Vec<Self::Route>, completion: C) -> Self;
}

impl<I, C> WorkerOutput<I, C> for WorkerBatch<I, C> {
    type Route = RoutedInput<I>;

    fn create(inputs: Vec<Self::Route>, completion: C) -> Self {
        Self { inputs, completion }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouterError {
    NoWorkers,
    WorkerUnavailable { worker_index: usize },
}

pub trait RoutableInput {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}
