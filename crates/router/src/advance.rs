use crossbeam_channel::Sender;

use crate::{AdvanceInput, AdvanceWorkerOutput, RouterError};

/// Broadcasts one horizon advancement to every worker without hashing.
pub fn route_advance<A, W>(advance: A, worker_outputs: &[Sender<W>]) -> Result<(), RouterError>
where
    A: AdvanceInput,
    W: AdvanceWorkerOutput<A::Time, A::Completion>,
{
    if worker_outputs.is_empty() {
        return Err(RouterError::NoWorkers);
    }

    let (time, completion) = advance.into_parts();
    let final_index = worker_outputs.len() - 1;
    let mut time = Some(time);
    let mut completion = Some(completion);
    for (worker_index, output) in worker_outputs.iter().enumerate() {
        let (worker_time, worker_completion) = if worker_index == final_index {
            (
                time.take().expect("final worker takes the advancement time"),
                completion.take().expect("final worker takes the completion handle"),
            )
        } else {
            (
                time.as_ref().expect("advancement time exists before final worker").clone(),
                completion.as_ref().expect("completion exists before final worker").clone(),
            )
        };
        output.send(W::advance(worker_time, worker_completion)).map_err(|_| RouterError::WorkerUnavailable { worker_index })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crossbeam_channel::{unbounded, Sender, TryRecvError};

    use crate::{AdvanceInput, AdvanceWorkerOutput, RouterError};

    struct Advance<C> {
        time: u64,
        completion: C,
    }

    impl<C> AdvanceInput for Advance<C>
    where
        C: Clone,
    {
        type Time = u64;
        type Completion = C;

        fn into_parts(self) -> (u64, C) {
            (self.time, self.completion)
        }
    }

    struct WorkerAdvance<C> {
        time: u64,
        completion: C,
    }

    impl<C> AdvanceWorkerOutput<u64, C> for WorkerAdvance<C> {
        fn advance(time: u64, completion: C) -> Self {
            Self { time, completion }
        }
    }

    #[derive(Default)]
    struct CloneCount(Arc<AtomicUsize>);

    impl Clone for CloneCount {
        fn clone(&self) -> Self {
            self.0.fetch_add(1, Ordering::Relaxed);
            Self(Arc::clone(&self.0))
        }
    }

    #[test]
    fn zero_workers_is_rejected() {
        assert_eq!(
            super::route_advance(Advance { time: 50, completion: () }, &[] as &[Sender<WorkerAdvance<()>>]),
            Err(RouterError::NoWorkers)
        );
    }

    #[test]
    fn one_worker_takes_the_original_completion_without_cloning() {
        let (worker, output) = unbounded::<WorkerAdvance<CloneCount>>();
        let completion = CloneCount::default();
        let clones = Arc::clone(&completion.0);

        super::route_advance(Advance { time: 50, completion }, &[worker]).unwrap();

        assert_eq!(output.recv().unwrap().time, 50);
        assert_eq!(clones.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn advancement_is_broadcast_to_every_worker() {
        let channels = (0..4).map(|_| unbounded()).collect::<Vec<_>>();
        let workers = channels.iter().map(|(sender, _)| sender.clone()).collect::<Vec<_>>();
        let (completion, done) = unbounded();

        super::route_advance(Advance { time: 50, completion }, &workers).unwrap();
        drop(workers);

        for (_, receiver) in channels {
            let message: WorkerAdvance<Sender<()>> = receiver.recv().unwrap();
            assert_eq!(message.time, 50);
            drop(message.completion);
        }
        assert_eq!(done.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn unavailable_worker_is_reported() {
        let (first, _first_output) = unbounded();
        let (second, second_output) = unbounded::<WorkerAdvance<()>>();
        drop(second_output);

        assert_eq!(
            super::route_advance(Advance { time: 50, completion: () }, &[first, second]),
            Err(RouterError::WorkerUnavailable { worker_index: 1 })
        );
    }
}
