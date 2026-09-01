use crossbeam_channel::Sender;

use crate::hash::RouterHasher;
use crate::{EventQueryInput, EventQueryWorkerOutput, RouterError, SnapshotQueryInput, SnapshotQueryWorkerOutput};

/// Partitions a historical snapshot query into one message per affected worker.
pub fn route_snapshot_query<Q, W>(seed: u64, query: Q, worker_outputs: &[Sender<W>]) -> Result<(), RouterError>
where
    Q: SnapshotQueryInput,
    Q::Time: Clone,
    W: SnapshotQueryWorkerOutput<Q::Time, Q::Response>,
{
    if worker_outputs.is_empty() {
        return Err(RouterError::NoWorkers);
    }

    let worker_count = worker_outputs.len();
    let hasher = RouterHasher::new(seed);
    let (time, snapshot_ids, response) = query.into_parts();
    let base_capacity = snapshot_ids.len().div_ceil(worker_count);
    let mut partitions = Vec::with_capacity(worker_count);
    partitions.resize_with(worker_count, || None::<Vec<u128>>);

    for snapshot_id in snapshot_ids {
        let worker_index = hasher.worker_index(snapshot_id, worker_count);
        partitions[worker_index].get_or_insert_with(|| Vec::with_capacity(base_capacity.saturating_add(1))).push(snapshot_id);
    }

    let mut remaining = partitions.iter().flatten().count();
    let mut time = Some(time);
    let mut response = Some(response);
    for (worker_index, snapshot_ids) in partitions.into_iter().enumerate() {
        let Some(snapshot_ids) = snapshot_ids else { continue };
        remaining -= 1;
        let worker_response = if remaining == 0 {
            response.take().expect("final worker takes the response")
        } else {
            response.as_ref().expect("response exists before final worker").clone()
        };
        let worker_time = if remaining == 0 {
            time.take().expect("final worker takes the query time")
        } else {
            time.as_ref().expect("time exists before final worker").clone()
        };
        worker_outputs[worker_index]
            .send(W::snapshot_query(worker_time, snapshot_ids, worker_response))
            .map_err(|_| RouterError::WorkerUnavailable { worker_index })?;
    }
    Ok(())
}

/// Routes a single-history event query to exactly one worker.
pub fn route_event_query<Q, W>(seed: u64, query: Q, worker_outputs: &[Sender<W>]) -> Result<(), RouterError>
where
    Q: EventQueryInput,
    W: EventQueryWorkerOutput<Q::Time, Q::Response>,
{
    if worker_outputs.is_empty() {
        return Err(RouterError::NoWorkers);
    }

    let (snapshot_id, from, to, response) = query.into_parts();
    let worker_index = RouterHasher::new(seed).worker_index(snapshot_id, worker_outputs.len());
    worker_outputs[worker_index]
        .send(W::event_query(snapshot_id, from, to, response))
        .map_err(|_| RouterError::WorkerUnavailable { worker_index })
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::{unbounded, Sender};

    use crate::{
        route_event_query, route_messages, route_snapshot_query, AdvanceInput, AdvanceWorkerOutput, EventQueryInput,
        EventQueryWorkerOutput, InputBatch, RoutableInput, RouteInput, RouteInputKind, RouteOutput, SnapshotQueryInput,
        SnapshotQueryWorkerOutput, WorkerOutput,
    };

    #[derive(Clone)]
    struct Response(Sender<Vec<u64>>);

    struct SnapshotQuery {
        time: u64,
        snapshot_ids: Vec<u128>,
        response: Response,
    }

    impl SnapshotQueryInput for SnapshotQuery {
        type Time = u64;
        type Response = Response;

        fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Response) {
            (self.time, self.snapshot_ids, self.response)
        }
    }

    struct EventQuery {
        snapshot_id: u128,
        from: u64,
        to: u64,
        response: Response,
    }

    impl EventQueryInput for EventQuery {
        type Time = u64;
        type Response = Response;

        fn into_parts(self) -> (u128, Self::Time, Self::Time, Self::Response) {
            (self.snapshot_id, self.from, self.to, self.response)
        }
    }

    enum WorkerMessage {
        Apply { count: usize },
        Snapshots { time: u64, snapshot_ids: Vec<u128>, response: Response },
        Events { snapshot_id: u128, from: u64, to: u64, response: Response },
        Advance { time: u64, completion: Response },
    }

    #[derive(Clone)]
    struct ApplyEvent(u128);

    impl RoutableInput for ApplyEvent {
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(self.0);
        }
    }

    struct ApplyRoute;

    impl RouteOutput<ApplyEvent> for ApplyRoute {
        fn create(_snapshot_id: u128, _input: ApplyEvent) -> Self {
            Self
        }
    }

    impl WorkerOutput<ApplyEvent, Response> for WorkerMessage {
        type Route = ApplyRoute;

        fn create(inputs: Vec<Self::Route>, _completion: Response) -> Self {
            Self::Apply { count: inputs.len() }
        }
    }

    enum RouterMessage {
        Apply(InputBatch<ApplyEvent, Response>),
        Snapshots(SnapshotQuery),
        Events(EventQuery),
        Advance(Advance),
    }

    struct Advance {
        time: u64,
        completion: Response,
    }

    impl AdvanceInput for Advance {
        type Time = u64;
        type Completion = Response;

        fn into_parts(self) -> (u64, Response) {
            (self.time, self.completion)
        }
    }

    impl RouteInput for RouterMessage {
        type Apply = InputBatch<ApplyEvent, Response>;
        type SnapshotQuery = SnapshotQuery;
        type EventQuery = EventQuery;
        type Advance = Advance;

        fn into_kind(self) -> RouteInputKind<InputBatch<ApplyEvent, Response>, SnapshotQuery, EventQuery, Advance> {
            match self {
                Self::Apply(batch) => RouteInputKind::Apply(batch),
                Self::Snapshots(query) => RouteInputKind::SnapshotQuery(query),
                Self::Events(query) => RouteInputKind::EventQuery(query),
                Self::Advance(advance) => RouteInputKind::Advance(advance),
            }
        }
    }

    impl SnapshotQueryWorkerOutput<u64, Response> for WorkerMessage {
        fn snapshot_query(time: u64, snapshot_ids: Vec<u128>, response: Response) -> Self {
            Self::Snapshots { time, snapshot_ids, response }
        }
    }

    impl EventQueryWorkerOutput<u64, Response> for WorkerMessage {
        fn event_query(snapshot_id: u128, from: u64, to: u64, response: Response) -> Self {
            Self::Events { snapshot_id, from, to, response }
        }
    }

    impl AdvanceWorkerOutput<u64, Response> for WorkerMessage {
        fn advance(time: u64, completion: Response) -> Self {
            Self::Advance { time, completion }
        }
    }

    #[test]
    fn snapshot_queries_are_partitioned_once_per_affected_worker() {
        let worker_channels = (0..4).map(|_| unbounded()).collect::<Vec<_>>();
        let workers = worker_channels.iter().map(|(sender, _)| sender.clone()).collect::<Vec<_>>();
        let (response, _results) = unbounded();

        route_snapshot_query(7, SnapshotQuery { time: 42, snapshot_ids: vec![1, 2, 3, 4, 5, 6], response: Response(response) }, &workers)
            .unwrap();

        let mut ids = Vec::new();
        for (_, receiver) in worker_channels {
            let messages = receiver.try_iter().collect::<Vec<_>>();
            assert!(messages.len() <= 1);
            for message in messages {
                let WorkerMessage::Snapshots { time, snapshot_ids, response } = message else { panic!("wrong message") };
                assert_eq!(time, 42);
                let _ = response.0;
                ids.extend(snapshot_ids);
            }
        }
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn event_queries_route_to_exactly_one_worker() {
        let worker_channels = (0..4).map(|_| unbounded()).collect::<Vec<_>>();
        let workers = worker_channels.iter().map(|(sender, _)| sender.clone()).collect::<Vec<_>>();
        let (response, _results) = unbounded();

        route_event_query(7, EventQuery { snapshot_id: 99, from: 10, to: 20, response: Response(response) }, &workers).unwrap();

        let messages = worker_channels.into_iter().flat_map(|(_, receiver)| receiver.try_iter().collect::<Vec<_>>()).collect::<Vec<_>>();
        assert_eq!(messages.len(), 1);
        let WorkerMessage::Events { snapshot_id, from, to, response } = messages.into_iter().next().unwrap() else {
            panic!("wrong message")
        };
        assert_eq!((snapshot_id, from, to), (99, 10, 20));
        let _ = response.0;
    }

    #[test]
    fn one_router_queue_dispatches_apply_and_both_query_kinds() {
        let (input, receiver) = unbounded();
        let (worker, output) = unbounded::<WorkerMessage>();
        let (response, _results) = unbounded();
        input.send(RouterMessage::Apply(InputBatch { inputs: vec![ApplyEvent(7)], completion: Response(response.clone()) })).unwrap();
        input
            .send(RouterMessage::Snapshots(SnapshotQuery { time: 42, snapshot_ids: vec![7], response: Response(response.clone()) }))
            .unwrap();
        input.send(RouterMessage::Events(EventQuery { snapshot_id: 7, from: 10, to: 20, response: Response(response) })).unwrap();
        let (advance_response, _advance_results) = unbounded();
        input.send(RouterMessage::Advance(Advance { time: 50, completion: Response(advance_response) })).unwrap();
        drop(input);

        route_messages(9, receiver, &[worker]).unwrap();

        let messages = output.try_iter().collect::<Vec<_>>();
        assert_eq!(messages.len(), 4);
        assert!(matches!(messages[0], WorkerMessage::Apply { count: 1 }));
        assert!(matches!(messages[1], WorkerMessage::Snapshots { .. }));
        assert!(matches!(messages[2], WorkerMessage::Events { .. }));
        let WorkerMessage::Advance { time, completion } = &messages[3] else { panic!("wrong message") };
        assert_eq!(*time, 50);
        let _ = &completion.0;
    }
}
