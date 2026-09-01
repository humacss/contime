use crossbeam_channel::Sender;

use crate::{ApiError, ConTime, Input, SnapshotListenerMessage};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    pub fn send_listen_snapshots(
        &self,
        time: I::Time,
        snapshot_ids: impl IntoIterator<Item = u128>,
        notifications: Sender<SnapshotListenerMessage<I::Time>>,
    ) -> Result<(), ApiError> {
        contime_api::send_listen_snapshots(self.runtime.input(), time, snapshot_ids, notifications)
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::Criterion;
    use crossbeam_channel::unbounded;

    use crate::{SnapshotListener, SnapshotListenerMessage};
    use contime_worker::SnapshotListener as WorkerSnapshotListener;

    #[test]
    fn concrete_listener_reports_registration_and_completed_replay() {
        let (notifications, received) = unbounded();
        let listener = SnapshotListener::new(notifications);

        assert!(WorkerSnapshotListener::registered(&listener, 5, vec![7, 11]));
        assert!(WorkerSnapshotListener::replayed(&listener, 5, vec![7]));
        assert_eq!(
            received.try_iter().collect::<Vec<_>>(),
            vec![
                SnapshotListenerMessage::Registered { time: 5, snapshot_ids: vec![7, 11] },
                SnapshotListenerMessage::Replayed { time: 5, snapshot_ids: vec![7] },
            ]
        );
    }

    #[test]
    fn concrete_listener_reports_a_disconnected_receiver() {
        let (notifications, received) = unbounded();
        let listener = SnapshotListener::new(notifications);
        drop(received);

        assert!(!WorkerSnapshotListener::registered(&listener, 5, vec![7]));
        assert!(!WorkerSnapshotListener::replayed(&listener, 5, vec![7]));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_listener_notification() {
        let mut criterion = Criterion::default();
        let (notifications, received) = unbounded();
        let listener = SnapshotListener::new(notifications);
        criterion.bench_function("core/listen/replayed_notification", |bencher| {
            bencher.iter(|| {
                assert!(WorkerSnapshotListener::replayed(black_box(&listener), black_box(5), black_box(vec![7])));
                black_box(received.recv().unwrap());
            });
        });
        criterion.final_summary();
    }
}
