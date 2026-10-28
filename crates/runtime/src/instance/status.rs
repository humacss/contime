//! Read-only observations through the existing private worker queues.

use crate::contime_runtime::{ApiError, ConTime, WorkerMessage, WorkerStatus};
use crossbeam_channel::{unbounded, Receiver};

impl<I: crate::contime_runtime::ThreadInput, S, W> ConTime<I, S, W> {
    /// Number of worker threads owned by this instance.
    pub fn worker_count(&self) -> usize {
        self.worker_inputs.len()
    }

    /// Requests one read-only report from each worker without waiting for replies.
    ///
    /// Each worker observes its own state when it handles the request. Reports
    /// are not a synchronized global snapshot or a fence for admission/router
    /// work. No replay, snapshot reconstruction, or pruning is requested.
    /// The dedicated response stream closes after all requests have been handled
    /// or dropped. Collectors must reject incomplete sets if a worker exits.
    /// Dropping the receiver cancels observation without blocking workers.
    ///
    /// # Errors
    /// Returns [`ApiError::OutputChannelClosed`] if a worker queue is closed.
    pub fn request_worker_status(&self) -> Result<Receiver<WorkerStatus<I::Time>>, ApiError> {
        let (response, reports) = unbounded();
        for (worker_id, input) in self.worker_inputs.iter().enumerate() {
            input.send(WorkerMessage::Status { worker_id, response: response.clone() }).map_err(|_| ApiError::OutputChannelClosed)?;
        }
        Ok(reports)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contime_runtime::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Event, Snapshot};
    use crate::contime_runtime::testing::Time;
    use crate::contime_runtime::{ConTimeConfig, Input, ThreadOutcome};
    use std::time::Duration;

    struct Update;
    impl Event for Update {
        type Time = Time;
        fn time(&self) -> Time {
            Time(1)
        }
    }
    impl Input for Update {
        fn event_id(&self) -> u128 {
            1
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }
    #[derive(Clone, Default)]
    struct Counter {
        time: Time,
        count: usize,
    }
    impl Snapshot for Counter {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.time
        }
        fn set_time(&mut self, time: Time) {
            self.time = time;
        }
    }
    impl ApplyEvents<Update> for Counter {
        fn create(_: u128, _: &Update) -> Self {
            Self::default()
        }
        fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Update>) {
            self.count += batch.events.count();
        }
    }
    fn start(workers: usize) -> ConTime<Update, Counter, ()> {
        ConTime::start(
            ConTimeConfig {
                router_count: 1,
                worker_count: workers,
                placement: crate::contime_runtime::Placement::default(),
                pruning_interval: Duration::from_millis(10),
                history_retention: Time(100),
                checkpoints: CheckpointConfig { interval: 128 },
            },
            (),
        )
        .unwrap()
    }
    fn reports(runtime: &ConTime<Update, Counter, ()>) -> Vec<WorkerStatus<Time>> {
        let receiver = runtime.request_worker_status().unwrap();
        let mut reports = Vec::new();
        loop {
            match receiver.recv_timeout(Duration::from_secs(2)) {
                Ok(status) => reports.push(status),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                Err(error) => panic!("status timed out: {error}"),
            }
        }
        reports.sort_by_key(|s| s.worker_id);
        reports
    }
    #[test]
    fn status_reports_each_worker_and_receivers_do_not_keep_shutdown_alive() {
        for workers in [1, 4] {
            let runtime = start(workers);
            assert_eq!(runtime.worker_count(), workers);
            let statuses = reports(&runtime);
            assert_eq!(statuses.iter().map(|s| s.worker_id).collect::<Vec<_>>(), (0..workers).collect::<Vec<_>>());
            assert!(statuses.iter().all(|s| s.history_stores == 0 && s.metadata_only == 0));
            drop(runtime.request_worker_status().unwrap());
            let retained_receiver = runtime.request_worker_status().unwrap();
            let shutdown = runtime.shutdown();
            assert_eq!(shutdown.workers.len(), workers);
            assert!(shutdown.workers.iter().all(|outcome| matches!(outcome, ThreadOutcome::Completed)));
            assert_eq!(retained_receiver.into_iter().count(), workers);
        }
    }
    #[test]
    fn status_counts_listener_metadata_separately_and_does_not_advance_replay() {
        let runtime = start(1);
        let (notify, notifications) = unbounded();
        runtime.send_listen_snapshots(Time(0), [9], notify).unwrap();
        notifications.recv_timeout(Duration::from_secs(2)).unwrap();
        let metadata = reports(&runtime);
        assert_eq!((metadata[0].history_stores, metadata[0].metadata_only), (0, 1));
        let (done, inserted) = unbounded();
        runtime.send([Update], done).unwrap();
        assert_eq!(inserted.recv_timeout(Duration::from_secs(2)), Err(crossbeam_channel::RecvTimeoutError::Disconnected));
        let pending = reports(&runtime);
        assert_eq!((pending[0].history_stores, pending[0].metadata_only), (1, 1));
        assert_eq!(pending[0].processing_target, Time(0));
        assert_eq!((pending[0].pending_histories, pending[0].due_histories), (1, 0));
        runtime.advance_to(Time(1)).unwrap();
        runtime.wait_until_idle(Duration::from_secs(2)).unwrap();
        let complete = reports(&runtime);
        assert_eq!(complete[0].processing_target, Time(1));
        assert_eq!((complete[0].pending_histories, complete[0].due_histories), (0, 0));
        assert_eq!(runtime.query_at(Time(1), [7]).unwrap()[0].count, 1);
        runtime.shutdown();
    }
    #[test]
    fn a_closed_worker_queue_returns_an_error() {
        let mut runtime = start(1);
        let (sender, receiver) = unbounded();
        drop(receiver);
        runtime.worker_inputs[0] = sender;
        assert!(matches!(runtime.request_worker_status(), Err(ApiError::OutputChannelClosed)));
        assert_eq!(runtime.shutdown().workers, vec![ThreadOutcome::Completed]);
    }
}
