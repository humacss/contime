use crossbeam_channel::Sender;

use crate::hash::RouterHasher;
use crate::{RouterError, SnapshotListenInput, SnapshotListenWorkerOutput};

/// Partitions snapshot-listener registrations by owning worker.
pub fn route_snapshot_listeners<R, W>(seed: u64, registration: R, worker_outputs: &[Sender<W>]) -> Result<(), RouterError>
where
    R: SnapshotListenInput,
    W: SnapshotListenWorkerOutput<R::Time, R::Listener>,
{
    if worker_outputs.is_empty() {
        return Err(RouterError::NoWorkers);
    }

    let worker_count = worker_outputs.len();
    let hasher = RouterHasher::new(seed);
    let (time, snapshot_ids, listener) = registration.into_parts();
    let base_capacity = snapshot_ids.len().div_ceil(worker_count);
    let mut partitions = Vec::with_capacity(worker_count);
    partitions.resize_with(worker_count, || None::<Vec<u128>>);

    for snapshot_id in snapshot_ids {
        let worker_index = hasher.worker_index(snapshot_id, worker_count);
        partitions[worker_index].get_or_insert_with(|| Vec::with_capacity(base_capacity.saturating_add(1))).push(snapshot_id);
    }

    let mut remaining = partitions.iter().flatten().count();
    let mut time = Some(time);
    let mut listener = Some(listener);
    for (worker_index, snapshot_ids) in partitions.into_iter().enumerate() {
        let Some(snapshot_ids) = snapshot_ids else { continue };
        remaining -= 1;
        let worker_listener = if remaining == 0 {
            listener.take().expect("final worker takes the listener")
        } else {
            listener.as_ref().expect("listener exists before final worker").clone()
        };
        let worker_time = if remaining == 0 {
            time.take().expect("final worker takes the watched time")
        } else {
            time.as_ref().expect("watched time exists before final worker").clone()
        };
        worker_outputs[worker_index]
            .send(W::listen(worker_time, snapshot_ids, worker_listener))
            .map_err(|_| RouterError::WorkerUnavailable { worker_index })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, Sender};

    use super::route_snapshot_listeners;
    use crate::{SnapshotListenInput, SnapshotListenWorkerOutput};

    #[derive(Clone)]
    struct Listener(Sender<u128>);

    struct Registration {
        time: u64,
        snapshot_ids: Vec<u128>,
        listener: Listener,
    }

    impl SnapshotListenInput for Registration {
        type Time = u64;
        type Listener = Listener;

        fn into_parts(self) -> (Self::Time, Vec<u128>, Self::Listener) {
            (self.time, self.snapshot_ids, self.listener)
        }
    }

    struct WorkerRegistration {
        time: u64,
        snapshot_ids: Vec<u128>,
        listener: Listener,
    }

    impl SnapshotListenWorkerOutput<u64, Listener> for WorkerRegistration {
        fn listen(time: u64, snapshot_ids: Vec<u128>, listener: Listener) -> Self {
            Self { time, snapshot_ids, listener }
        }
    }

    #[test]
    fn partitions_each_snapshot_id_to_exactly_one_worker() {
        let workers = (0..4).map(|_| unbounded::<WorkerRegistration>()).collect::<Vec<_>>();
        let outputs = workers.iter().map(|(sender, _)| sender.clone()).collect::<Vec<_>>();
        let (listener, notifications) = unbounded();

        route_snapshot_listeners(
            7,
            Registration { time: 55, snapshot_ids: vec![1, 2, 3, 4, 5, 6], listener: Listener(listener) },
            &outputs,
        )
        .unwrap();

        let mut routed = Vec::new();
        for (_, receiver) in workers {
            let messages = receiver.try_iter().collect::<Vec<_>>();
            assert!(messages.len() <= 1);
            for message in messages {
                assert_eq!(message.time, 55);
                for snapshot_id in message.snapshot_ids {
                    message.listener.0.send(snapshot_id).unwrap();
                    routed.push(snapshot_id);
                }
            }
        }
        routed.sort_unstable();
        assert_eq!(routed, vec![1, 2, 3, 4, 5, 6]);
        let mut observed = notifications.try_iter().collect::<Vec<_>>();
        observed.sort_unstable();
        assert_eq!(observed, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn empty_registrations_emit_no_worker_messages() {
        let workers = (0..4).map(|_| unbounded::<WorkerRegistration>()).collect::<Vec<_>>();
        let outputs = workers.iter().map(|(sender, _)| sender.clone()).collect::<Vec<_>>();
        let (listener, notifications) = unbounded();

        route_snapshot_listeners(7, Registration { time: 55, snapshot_ids: Vec::new(), listener: Listener(listener) }, &outputs).unwrap();

        assert!(workers.into_iter().all(|(_, receiver)| receiver.try_recv().is_err()));
        assert!(notifications.try_recv().is_err());
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_snapshot_listener_routing() {
        let mut criterion = Criterion::default();
        for worker_count in [1, 8] {
            criterion.bench_function(&format!("router/listen/1000_ids/{worker_count}_workers"), |bencher| {
                bencher.iter_batched(
                    || {
                        let workers = (0..worker_count).map(|_| unbounded::<WorkerRegistration>()).collect::<Vec<_>>();
                        let outputs = workers.iter().map(|(sender, _)| sender.clone()).collect::<Vec<_>>();
                        let (listener, notifications) = unbounded();
                        let registration = Registration { time: 55, snapshot_ids: (0..1_000).collect(), listener: Listener(listener) };
                        (workers, outputs, notifications, registration)
                    },
                    |(workers, outputs, notifications, registration)| {
                        route_snapshot_listeners(7, registration, &outputs).unwrap();
                        black_box((workers, notifications));
                    },
                    BatchSize::LargeInput,
                );
            });
        }
        criterion.final_summary();
    }
}
