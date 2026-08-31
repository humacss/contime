use crossbeam_channel::{Receiver, Sender};

use crate::hash::RouterHasher;
use crate::types::{InputBatch, RoutableInput, RoutedInput, RouterError, WorkerBatch};

trait Deps<I, C> {
    fn worker_count(&self) -> usize;
    fn send(&self, worker_index: usize, batch: WorkerBatch<I, C>) -> Result<(), ()>;
}

struct DefaultDeps<'a, I, C> {
    worker_outputs: &'a [Sender<WorkerBatch<I, C>>],
}

impl<I, C> Deps<I, C> for DefaultDeps<'_, I, C> {
    fn worker_count(&self) -> usize {
        self.worker_outputs.len()
    }

    fn send(&self, worker_index: usize, batch: WorkerBatch<I, C>) -> Result<(), ()> {
        self.worker_outputs[worker_index].send(batch).map_err(|_| ())
    }
}

pub fn route<I, C>(seed: u64, input: Receiver<InputBatch<I, C>>, worker_outputs: &[Sender<WorkerBatch<I, C>>]) -> Result<(), RouterError>
where
    I: RoutableInput + Clone,
    C: Clone,
{
    route_with_deps(seed, input, &DefaultDeps { worker_outputs })
}

fn route_with_deps<D, I, C>(seed: u64, input: Receiver<InputBatch<I, C>>, deps: &D) -> Result<(), RouterError>
where
    D: Deps<I, C>,
    I: RoutableInput + Clone,
    C: Clone,
{
    if deps.worker_count() == 0 {
        return Err(RouterError::NoWorkers);
    }

    let hasher = RouterHasher::new(seed);
    while let Ok(batch) = input.recv() {
        route_batch(&hasher, batch, deps)?;
    }
    Ok(())
}

fn route_batch<D, I, C>(hasher: &RouterHasher, batch: InputBatch<I, C>, deps: &D) -> Result<(), RouterError>
where
    D: Deps<I, C>,
    I: RoutableInput + Clone,
    C: Clone,
{
    let worker_count = deps.worker_count();
    let base_capacity = batch.inputs.len().div_ceil(worker_count);
    let estimated_capacity = base_capacity.saturating_add(base_capacity / 4).saturating_add(1);
    let mut worker_inputs = Vec::with_capacity(worker_count);
    worker_inputs.resize_with(worker_count, || None::<Vec<RoutedInput<I>>>);

    for input in batch.inputs {
        let mut pending_snapshot_id = None;
        input.snapshot_ids(&mut |snapshot_id| {
            if let Some(previous_snapshot_id) = pending_snapshot_id.replace(snapshot_id) {
                push_route(&mut worker_inputs, hasher, worker_count, estimated_capacity, previous_snapshot_id, input.clone());
            }
        });
        if let Some(final_snapshot_id) = pending_snapshot_id {
            push_route(&mut worker_inputs, hasher, worker_count, estimated_capacity, final_snapshot_id, input);
        }
    }

    let affected_workers = worker_inputs.iter().flatten().count();
    let mut remaining_workers = affected_workers;
    let mut completion = Some(batch.completion);

    for (worker_index, inputs) in worker_inputs.into_iter().enumerate() {
        let Some(inputs) = inputs else {
            continue;
        };
        remaining_workers -= 1;
        let worker_completion = if remaining_workers == 0 {
            completion.take().expect("the final affected worker takes the completion handle")
        } else {
            completion.as_ref().expect("the completion handle exists before the final worker").clone()
        };
        deps.send(worker_index, WorkerBatch { inputs, completion: worker_completion })
            .map_err(|()| RouterError::WorkerUnavailable { worker_index })?;
    }

    Ok(())
}

fn push_route<I>(
    worker_inputs: &mut [Option<Vec<RoutedInput<I>>>],
    hasher: &RouterHasher,
    worker_count: usize,
    estimated_capacity: usize,
    snapshot_id: u128,
    input: I,
) {
    let worker_index = hasher.worker_index(snapshot_id, worker_count);
    worker_inputs[worker_index].get_or_insert_with(|| Vec::with_capacity(estimated_capacity)).push(RoutedInput { snapshot_id, input });
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, Receiver, Sender};

    use super::route;
    use crate::hash::RouterHasher;
    use crate::{InputBatch, RoutableInput, RouterError, WorkerBatch};

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestInput {
        value: u64,
        snapshot_ids: Vec<u128>,
    }

    impl RoutableInput for TestInput {
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            self.snapshot_ids.iter().copied().for_each(emit);
        }
    }

    struct SharedInput<I>(Arc<I>);

    impl<I> Clone for SharedInput<I> {
        fn clone(&self) -> Self {
            Self(Arc::clone(&self.0))
        }
    }

    impl<I> RoutableInput for SharedInput<I>
    where
        I: RoutableInput,
    {
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            self.0.snapshot_ids(emit);
        }
    }

    #[test]
    fn route_preserves_the_selected_input_ownership_type() {
        let (input_sender, input_receiver) = unbounded();
        let (worker_sender, worker_receiver) = unbounded();
        input_sender.send(InputBatch { inputs: vec![TestInput { value: 10, snapshot_ids: vec![11] }], completion: () }).unwrap();
        drop(input_sender);

        route(7, input_receiver, &[worker_sender]).unwrap();

        let input: TestInput = worker_receiver.recv().unwrap().inputs.pop().unwrap().input;
        assert_eq!(input.value, 10);
    }

    struct NonCloneInput {
        value: u64,
        snapshot_ids: Vec<u128>,
    }

    impl RoutableInput for NonCloneInput {
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            self.snapshot_ids.iter().copied().for_each(emit);
        }
    }

    #[test]
    fn route_accepts_a_non_clone_event_behind_a_cloneable_wrapper() {
        let (input_sender, input_receiver) = unbounded();
        let (worker_sender, worker_receiver) = unbounded();
        input_sender
            .send(InputBatch { inputs: vec![SharedInput(Arc::new(NonCloneInput { value: 10, snapshot_ids: vec![11] }))], completion: () })
            .unwrap();
        drop(input_sender);

        route(7, input_receiver, &[worker_sender]).unwrap();

        let routed = worker_receiver.recv().unwrap().inputs.pop().unwrap();
        assert_eq!(routed.snapshot_id, 11);
        assert_eq!(routed.input.0.value, 10);
    }

    fn route_once(seed: u64, inputs: Vec<TestInput>, worker_count: usize) -> Vec<(usize, WorkerBatch<TestInput, ()>)> {
        let (input_sender, input_receiver) = unbounded();
        input_sender.send(InputBatch { inputs, completion: () }).unwrap();
        drop(input_sender);

        let mut worker_outputs = Vec::with_capacity(worker_count);
        let mut worker_receivers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let (worker_sender, worker_receiver) = unbounded();
            worker_outputs.push(worker_sender);
            worker_receivers.push(worker_receiver);
        }

        route(seed, input_receiver, &worker_outputs).unwrap();

        worker_receivers
            .into_iter()
            .enumerate()
            .flat_map(|(worker_index, receiver)| receiver.try_iter().map(move |batch| (worker_index, batch)).collect::<Vec<_>>())
            .collect()
    }

    fn snapshot_id_for_worker(seed: u64, worker_count: usize, target_worker: usize) -> u128 {
        let hasher = RouterHasher::new(seed);
        (0..).find(|snapshot_id| hasher.worker_index(*snapshot_id, worker_count) == target_worker).unwrap()
    }

    fn placements(batches: Vec<(usize, WorkerBatch<TestInput, ()>)>) -> Vec<(u128, usize)> {
        let mut placements = batches
            .into_iter()
            .flat_map(|(worker_index, batch)| batch.inputs.into_iter().map(move |input| (input.snapshot_id, worker_index)))
            .collect::<Vec<_>>();
        placements.sort_unstable();
        placements
    }

    #[test]
    fn route_rejects_an_empty_worker_list() {
        let (_input_sender, input_receiver) = unbounded::<InputBatch<TestInput, ()>>();

        let result = route(7, input_receiver, &[]);

        assert_eq!(result, Err(RouterError::NoWorkers));
    }

    #[test]
    fn route_stops_normally_when_input_disconnects() {
        let (input_sender, input_receiver) = unbounded::<InputBatch<TestInput, ()>>();
        let (worker_sender, _worker_receiver) = unbounded();
        drop(input_sender);

        let result = route(7, input_receiver, &[worker_sender]);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn route_dispatches_every_snapshot_route_once() {
        let batches = route_once(
            7,
            vec![
                TestInput { value: 10, snapshot_ids: vec![11] },
                TestInput { value: 20, snapshot_ids: vec![22] },
                TestInput { value: 30, snapshot_ids: vec![33] },
            ],
            4,
        );

        let mut routed = batches
            .into_iter()
            .flat_map(|(_worker_index, batch)| batch.inputs)
            .map(|routed| (routed.snapshot_id, routed.input.value))
            .collect::<Vec<_>>();
        routed.sort_unstable();

        assert_eq!(routed, vec![(11, 10), (22, 20), (33, 30)]);
    }

    #[test]
    fn identical_seeds_produce_identical_worker_assignments() {
        let inputs =
            (0..100).map(|snapshot_id| TestInput { value: snapshot_id as u64, snapshot_ids: vec![snapshot_id] }).collect::<Vec<_>>();

        let first = placements(route_once(7, inputs.clone(), 8));
        let second = placements(route_once(7, inputs, 8));

        assert_eq!(first, second);
    }

    #[test]
    fn route_preserves_input_order_within_a_worker_batch() {
        let snapshot_id = snapshot_id_for_worker(7, 4, 0);
        let batches = route_once(
            7,
            vec![
                TestInput { value: 10, snapshot_ids: vec![snapshot_id] },
                TestInput { value: 20, snapshot_ids: vec![snapshot_id] },
                TestInput { value: 30, snapshot_ids: vec![snapshot_id] },
            ],
            4,
        );

        let values = batches[0].1.inputs.iter().map(|routed| routed.input.value).collect::<Vec<_>>();
        assert_eq!(values, vec![10, 20, 30]);
    }

    #[test]
    fn additional_routes_clone_the_arc_once_each() {
        let event = Arc::new(TestInput { value: 10, snapshot_ids: vec![11, 22, 33] });
        let weak = Arc::downgrade(&event);
        let (input_sender, input_receiver) = unbounded();
        let mut worker_outputs = Vec::new();
        let mut worker_receivers = Vec::new();
        for _ in 0..4 {
            let (worker_sender, worker_receiver) = unbounded();
            worker_outputs.push(worker_sender);
            worker_receivers.push(worker_receiver);
        }
        input_sender.send(InputBatch { inputs: vec![SharedInput(event)], completion: () }).unwrap();
        drop(input_sender);

        route(7, input_receiver, &worker_outputs).unwrap();

        assert_eq!(weak.strong_count(), 3);
        let routed_count = worker_receivers.iter().flat_map(|receiver| receiver.try_iter()).map(|batch| batch.inputs.len()).sum::<usize>();
        assert_eq!(routed_count, 3);
    }

    #[test]
    fn one_route_moves_the_only_arc_without_cloning() {
        let event = Arc::new(TestInput { value: 10, snapshot_ids: vec![11] });
        let weak = Arc::downgrade(&event);
        let (input_sender, input_receiver) = unbounded();
        let (worker_sender, worker_receiver) = unbounded();
        input_sender.send(InputBatch { inputs: vec![SharedInput(event)], completion: () }).unwrap();
        drop(input_sender);

        route(7, input_receiver, &[worker_sender]).unwrap();

        assert_eq!(weak.strong_count(), 1);
        assert_eq!(worker_receiver.try_iter().map(|batch| batch.inputs.len()).sum::<usize>(), 1);
    }

    struct CompletionToken {
        clone_count: Arc<AtomicUsize>,
    }

    impl Clone for CompletionToken {
        fn clone(&self) -> Self {
            self.clone_count.fetch_add(1, Ordering::Relaxed);
            Self { clone_count: Arc::clone(&self.clone_count) }
        }
    }

    #[test]
    fn route_clones_completion_only_for_additional_workers() {
        let snapshot_ids = (0..3).map(|worker| snapshot_id_for_worker(7, 3, worker)).collect::<Vec<_>>();
        let completion_clone_count = Arc::new(AtomicUsize::new(0));
        let (input_sender, input_receiver) = unbounded();
        let mut worker_outputs = Vec::new();
        let mut worker_receivers = Vec::new();
        for _ in 0..3 {
            let (worker_sender, worker_receiver) = unbounded();
            worker_outputs.push(worker_sender);
            worker_receivers.push(worker_receiver);
        }
        input_sender
            .send(InputBatch {
                inputs: vec![TestInput { value: 10, snapshot_ids }],
                completion: CompletionToken { clone_count: Arc::clone(&completion_clone_count) },
            })
            .unwrap();
        drop(input_sender);

        route(7, input_receiver, &worker_outputs).unwrap();

        assert_eq!(completion_clone_count.load(Ordering::Relaxed), 2);
        assert!(worker_receivers.iter().all(|receiver| receiver.len() == 1));
    }

    #[test]
    fn route_sends_nothing_for_inputs_without_snapshot_ids() {
        let batches = route_once(7, vec![TestInput { value: 10, snapshot_ids: Vec::new() }], 4);

        assert!(batches.is_empty());
    }

    #[test]
    fn route_reports_the_disconnected_worker_index() {
        let selected_worker = 1;
        let snapshot_id = snapshot_id_for_worker(7, 2, selected_worker);
        let (input_sender, input_receiver) = unbounded();
        input_sender.send(InputBatch { inputs: vec![TestInput { value: 10, snapshot_ids: vec![snapshot_id] }], completion: () }).unwrap();
        drop(input_sender);
        let (first_worker, _first_receiver) = unbounded();
        let (second_worker, second_receiver) = unbounded();
        drop(second_receiver);

        let result = route(7, input_receiver, &[first_worker, second_worker]);

        assert_eq!(result, Err(RouterError::WorkerUnavailable { worker_index: selected_worker }));
    }

    type BenchmarkFixture = (
        Receiver<InputBatch<TestInput, Sender<()>>>,
        Vec<Sender<WorkerBatch<TestInput, Sender<()>>>>,
        Vec<Receiver<WorkerBatch<TestInput, Sender<()>>>>,
        Receiver<()>,
    );

    fn benchmark_fixture(input_count: usize, worker_count: usize) -> BenchmarkFixture {
        let inputs = (0..input_count)
            .map(|snapshot_id| TestInput { value: snapshot_id as u64, snapshot_ids: vec![snapshot_id as u128] })
            .collect::<Vec<_>>();
        let (completion, completion_receiver) = unbounded();
        let (input_sender, input_receiver) = unbounded();
        input_sender.send(InputBatch { inputs, completion }).unwrap();
        drop(input_sender);

        let mut worker_outputs = Vec::with_capacity(worker_count);
        let mut worker_receivers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let (worker_sender, worker_receiver) = unbounded();
            worker_outputs.push(worker_sender);
            worker_receivers.push(worker_receiver);
        }

        (input_receiver, worker_outputs, worker_receivers, completion_receiver)
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_route() {
        let mut criterion = Criterion::default();

        criterion.bench_function("router/1000_inputs/8_workers", |bencher| {
            bencher.iter_batched(
                || benchmark_fixture(1_000, 8),
                |(input_receiver, worker_outputs, worker_receivers, completion_receiver)| {
                    route(7, input_receiver, &worker_outputs).unwrap();
                    black_box((worker_outputs, worker_receivers, completion_receiver))
                },
                BatchSize::LargeInput,
            );
        });

        criterion.final_summary();
    }
}
