use crossbeam_channel::Sender;

use crate::Placement;
use crate::{EventQueryInput, EventQueryWorkerOutput, RouterError, SnapshotQueryInput, SnapshotQueryWorkerOutput};

/// Partitions a historical snapshot query into one message per affected worker.
pub fn route_snapshot_query<Q, W>(placement: Placement, query: Q, worker_outputs: &[Sender<W>]) -> Result<(), RouterError>
where
    Q: SnapshotQueryInput,
    Q::Time: Clone,
    W: SnapshotQueryWorkerOutput<Q::Time, Q::Response>,
{
    if worker_outputs.is_empty() {
        return Err(RouterError::NoWorkers);
    }

    let worker_count = worker_outputs.len();

    let (time, snapshot_ids, response) = query.into_parts();
    let base_capacity = snapshot_ids.len().div_ceil(worker_count);
    let mut partitions = Vec::with_capacity(worker_count);
    partitions.resize_with(worker_count, || None::<Vec<u128>>);

    for snapshot_id in snapshot_ids {
        let worker_index = placement.worker_index(snapshot_id, worker_count);
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
pub fn route_event_query<Q, W>(placement: Placement, query: Q, worker_outputs: &[Sender<W>]) -> Result<(), RouterError>
where
    Q: EventQueryInput,
    W: EventQueryWorkerOutput<Q::Time, Q::Response>,
{
    if worker_outputs.is_empty() {
        return Err(RouterError::NoWorkers);
    }

    let (snapshot_id, from, to, response) = query.into_parts();
    let worker_index = placement.worker_index(snapshot_id, worker_outputs.len());
    worker_outputs[worker_index]
        .send(W::event_query(snapshot_id, from, to, response))
        .map_err(|_| RouterError::WorkerUnavailable { worker_index })
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::{unbounded, Sender};

    use crate::{
        route_event_query, route_messages, route_snapshot_query, AdvanceInput, AdvanceWorkerOutput, EventQueryInput,
        EventQueryWorkerOutput, InputBatch, RoutableInput, RouteInput, RouteInputKind, RouteOutput, SnapshotListenInput,
        SnapshotListenWorkerOutput, SnapshotQueryInput, SnapshotQueryWorkerOutput, WorkerOutput,
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
        Fence { round: u64, router: usize },
        Snapshots { time: u64, snapshot_ids: Vec<u128>, response: Response },
        Events { snapshot_id: u128, from: u64, to: u64, response: Response },
        Listen { time: u64, snapshot_ids: Vec<u128>, listener: Response },
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
        GatedApply { batch: InputBatch<ApplyEvent, Response>, started: Sender<()>, release: crossbeam_channel::Receiver<()> },
        Fence { round: u64, observed: Sender<u64> },
        Snapshots(SnapshotQuery),
        Events(EventQuery),
        Listen(Listen),
        Advance(Advance),
    }

    struct Listen {
        time: u64,
        snapshot_ids: Vec<u128>,
        listener: Response,
    }

    impl SnapshotListenInput for Listen {
        type Time = u64;
        type Listener = Response;

        fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener) {
            (self.time, self.snapshot_ids, self.listener)
        }
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
        type SnapshotListen = Listen;
        type Advance = Advance;

        fn into_kind(self) -> RouteInputKind<InputBatch<ApplyEvent, Response>, SnapshotQuery, EventQuery, Listen, Advance> {
            match self {
                Self::Apply(batch) => RouteInputKind::Apply(batch),
                Self::GatedApply { batch, started, release } => {
                    started.send(()).unwrap();
                    release.recv().unwrap();
                    RouteInputKind::Apply(batch)
                }
                Self::Fence { round, observed } => RouteInputKind::Fence { round, observed },
                Self::Snapshots(query) => RouteInputKind::SnapshotQuery(query),
                Self::Events(query) => RouteInputKind::EventQuery(query),
                Self::Listen(listen) => RouteInputKind::SnapshotListen(listen),
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

    impl SnapshotListenWorkerOutput<u64, Response> for WorkerMessage {
        fn listen(time: u64, snapshot_ids: Vec<u128>, listener: Response) -> Self {
            Self::Listen { time, snapshot_ids, listener }
        }
    }

    impl crate::CoordinationOutput<u64, Response> for WorkerMessage {
        fn fence(round: u64, router: usize) -> Self {
            Self::Fence { round, router }
        }
        fn prune(time: u64, completion: Response) -> Self {
            Self::Advance { time, completion }
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

        route_snapshot_query(
            crate::Placement::default(),
            SnapshotQuery { time: 42, snapshot_ids: vec![1, 2, 3, 4, 5, 6], response: Response(response) },
            &workers,
        )
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

        route_event_query(
            crate::Placement::default(),
            EventQuery { snapshot_id: 99, from: 10, to: 20, response: Response(response) },
            &workers,
        )
        .unwrap();

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
        let (listen_response, _listen_results) = unbounded();
        input.send(RouterMessage::Listen(Listen { time: 42, snapshot_ids: vec![7], listener: Response(listen_response) })).unwrap();
        let (advance_response, _advance_results) = unbounded();
        input.send(RouterMessage::Advance(Advance { time: 50, completion: Response(advance_response) })).unwrap();
        drop(input);

        route_messages(crate::Placement::default(), receiver, &[worker], crossbeam_channel::never(), crossbeam_channel::never()).unwrap();

        let messages = output.try_iter().collect::<Vec<_>>();
        assert_eq!(messages.len(), 5);
        assert!(matches!(messages[0], WorkerMessage::Apply { count: 1 }));
        assert!(matches!(messages[1], WorkerMessage::Snapshots { .. }));
        assert!(matches!(messages[2], WorkerMessage::Events { .. }));
        let WorkerMessage::Listen { time, snapshot_ids, listener } = &messages[3] else { panic!("wrong message") };
        assert_eq!(*time, 42);
        assert_eq!(snapshot_ids, &[7]);
        let _ = &listener.0;
        let WorkerMessage::Advance { time, completion } = &messages[4] else { panic!("wrong message") };
        assert_eq!(*time, 50);
        let _ = &completion.0;
    }

    #[test]
    fn a_round_marker_is_observed_before_the_router_flushes_to_all_workers() {
        let (input, receiver) = unbounded();
        let (control, controls) = unbounded();
        let (observed, observations) = unbounded();
        let channels = (0..2).map(|_| unbounded::<WorkerMessage>()).collect::<Vec<_>>();
        let workers = channels.iter().map(|(sender, _)| sender.clone()).collect::<Vec<_>>();
        let (response, _results) = unbounded();
        input.send(RouterMessage::Apply(InputBatch { inputs: vec![ApplyEvent(0)], completion: Response(response) })).unwrap();
        input.send(RouterMessage::Fence { round: 3, observed }).unwrap();
        let thread = std::thread::spawn(move || {
            route_messages(crate::Placement::default(), receiver, &workers, crossbeam_channel::never(), controls)
        });
        assert_eq!(observations.recv_timeout(std::time::Duration::from_secs(2)).unwrap(), 3);
        control.send(crate::Flush { round: 3, router: 1 }).unwrap();
        assert!(matches!(channels[0].1.recv_timeout(std::time::Duration::from_secs(2)).unwrap(), WorkerMessage::Apply { count: 1 }));
        for (_, receiver) in &channels {
            assert!(matches!(
                receiver.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
                WorkerMessage::Fence { round: 3, router: 1 }
            ));
        }
        drop(input);
        thread.join().unwrap().unwrap();
    }

    #[test]
    fn a_second_router_observing_the_marker_cannot_flush_the_first_routers_active_dispatch() {
        let timeout = std::time::Duration::from_secs(2);
        let (input, receiver) = unbounded();
        let (worker, output) = unbounded::<WorkerMessage>();
        let (first_control, first_controls) = unbounded();
        let (second_control, second_controls) = unbounded();
        let first_receiver = receiver.clone();
        let first_worker = worker.clone();
        let first = std::thread::spawn(move || {
            route_messages(crate::Placement::default(), first_receiver, &[first_worker], crossbeam_channel::never(), first_controls)
        });
        let (started, active) = unbounded();
        let (release, gate) = unbounded();
        let (response, _) = unbounded();
        input
            .send(RouterMessage::GatedApply {
                batch: InputBatch { inputs: vec![ApplyEvent(0)], completion: Response(response) },
                started,
                release: gate,
            })
            .unwrap();
        active.recv_timeout(timeout).unwrap();
        let second = std::thread::spawn(move || {
            route_messages(crate::Placement::default(), receiver, &[worker], crossbeam_channel::never(), second_controls)
        });
        let (observed, observations) = unbounded();
        input.send(RouterMessage::Fence { round: 9, observed }).unwrap();
        assert_eq!(observations.recv_timeout(timeout).unwrap(), 9);
        first_control.send(crate::Flush { round: 9, router: 0 }).unwrap();
        second_control.send(crate::Flush { round: 9, router: 1 }).unwrap();
        assert!(matches!(output.recv_timeout(timeout).unwrap(), WorkerMessage::Fence { round: 9, router: 1 }));
        assert!(output.is_empty());
        release.send(()).unwrap();
        assert!(matches!(output.recv_timeout(timeout).unwrap(), WorkerMessage::Apply { count: 1 }));
        assert!(matches!(output.recv_timeout(timeout).unwrap(), WorkerMessage::Fence { round: 9, router: 0 }));
        drop(input);
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    }

    #[test]
    fn custom_placement_is_shared_by_applies_queries_and_listeners() {
        let (input, receiver) = unbounded();
        let channels = (0..3).map(|_| unbounded::<WorkerMessage>()).collect::<Vec<_>>();
        let workers = channels.iter().map(|(sender, _)| sender.clone()).collect::<Vec<_>>();
        let placement = crate::Placement::with_mapper(|id| id + 1);
        let (response, _results) = unbounded();
        input.send(RouterMessage::Apply(InputBatch { inputs: vec![ApplyEvent(7)], completion: Response(response.clone()) })).unwrap();
        input
            .send(RouterMessage::Snapshots(SnapshotQuery { time: 42, snapshot_ids: vec![7], response: Response(response.clone()) }))
            .unwrap();
        input.send(RouterMessage::Events(EventQuery { snapshot_id: 7, from: 10, to: 20, response: Response(response.clone()) })).unwrap();
        input.send(RouterMessage::Listen(Listen { time: 42, snapshot_ids: vec![7], listener: Response(response) })).unwrap();
        drop(input);
        route_messages(placement, receiver, &workers, crossbeam_channel::never(), crossbeam_channel::never()).unwrap();
        for (index, (_, receiver)) in channels.into_iter().enumerate() {
            let messages = receiver.try_iter().collect::<Vec<_>>();
            if index == 2 {
                assert_eq!(messages.len(), 4);
                assert!(matches!(messages[0], WorkerMessage::Apply { count: 1 }));
                assert!(matches!(messages[1], WorkerMessage::Snapshots { .. }));
                assert!(matches!(messages[2], WorkerMessage::Events { .. }));
                assert!(matches!(messages[3], WorkerMessage::Listen { .. }));
            } else {
                assert!(messages.is_empty());
            }
        }
    }

    #[test]
    fn activity_registration_waits_for_idle_and_notifies_every_listener() {
        let (input, receiver) = unbounded();
        let (worker, output) = crossbeam_channel::bounded::<WorkerMessage>(0);
        let (register, registrations) = unbounded();
        let thread = std::thread::spawn(move || {
            crate::route_messages(crate::Placement::default(), receiver, &[worker], registrations, crossbeam_channel::never())
        });
        let timeout = std::time::Duration::from_secs(2);
        let (first, first_reports) = unbounded();
        register.send(first).unwrap();
        assert!(!first_reports.recv_timeout(timeout).unwrap());
        let (response, _results) = unbounded();
        input.send(RouterMessage::Apply(InputBatch { inputs: vec![ApplyEvent(7)], completion: Response(response) })).unwrap();
        assert!(first_reports.recv_timeout(timeout).unwrap());
        // Dispatch cannot finish until the worker accepts this rendezvous send.
        let (second, second_reports) = unbounded();
        register.send(second).unwrap();
        assert!(matches!(second_reports.try_recv(), Err(crossbeam_channel::TryRecvError::Empty)));
        assert!(matches!(output.recv_timeout(timeout).unwrap(), WorkerMessage::Apply { count: 1 }));
        assert!(!first_reports.recv_timeout(timeout).unwrap());
        assert!(!second_reports.recv_timeout(timeout).unwrap());
        drop(input);
        thread.join().unwrap().unwrap();
    }
}
