use crossbeam_channel::Sender;

use crate::{ApiError, SnapshotListenOutput};

/// Forwards one asynchronous snapshot-listener registration.
pub fn send_listen_snapshots<O, T, N, Ids>(output: &Sender<O>, time: T, snapshot_ids: Ids, notifications: Sender<N>) -> Result<(), ApiError>
where
    O: SnapshotListenOutput<T, N>,
    Ids: IntoIterator<Item = u128>,
{
    let snapshot_ids = snapshot_ids.into_iter().collect::<Vec<_>>();
    if snapshot_ids.is_empty() {
        return Ok(());
    }

    output.send(O::listen(time, snapshot_ids, notifications)).map_err(|_| ApiError::OutputChannelClosed)
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, Sender, TryRecvError};

    use super::send_listen_snapshots;
    use crate::{ApiError, SnapshotListenOutput};

    struct ListenOutput {
        time: u64,
        snapshot_ids: Vec<u128>,
        notifications: Sender<u128>,
    }

    impl SnapshotListenOutput<u64, u128> for ListenOutput {
        fn listen(time: u64, snapshot_ids: Vec<u128>, notifications: Sender<u128>) -> Self {
            Self { time, snapshot_ids, notifications }
        }
    }

    #[test]
    fn forwards_all_snapshot_ids_with_the_consumers_sender() {
        let (output, requests) = unbounded::<ListenOutput>();
        let (notifications, received) = unbounded();

        send_listen_snapshots(&output, 55, [3, 5, 8], notifications).unwrap();

        let request = requests.recv().unwrap();
        assert_eq!(request.time, 55);
        assert_eq!(request.snapshot_ids, vec![3, 5, 8]);
        request.notifications.send(13).unwrap();
        assert_eq!(received.recv().unwrap(), 13);
    }

    #[test]
    fn empty_snapshot_id_sequences_are_not_forwarded() {
        let (output, requests) = unbounded::<ListenOutput>();
        let (notifications, received) = unbounded();

        send_listen_snapshots(&output, 55, [] as [u128; 0], notifications).unwrap();

        assert!(matches!(requests.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(received.try_recv().unwrap_err(), TryRecvError::Disconnected);
    }

    #[test]
    fn reports_a_closed_output_channel() {
        let (output, requests) = unbounded::<ListenOutput>();
        let (notifications, _received) = unbounded();
        drop(requests);

        assert_eq!(send_listen_snapshots(&output, 55, [3], notifications), Err(ApiError::OutputChannelClosed));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_send_listen_snapshots() {
        let mut criterion = Criterion::default();
        criterion.bench_function("api/send_listen_snapshots/1000_ids", |bencher| {
            bencher.iter_batched(
                || {
                    let snapshot_ids = (0..1_000).collect::<Vec<_>>();
                    let (output, requests) = unbounded::<ListenOutput>();
                    let (notifications, received) = unbounded();
                    (snapshot_ids, output, requests, notifications, received)
                },
                |(snapshot_ids, output, requests, notifications, received)| {
                    send_listen_snapshots(&output, 55, snapshot_ids, notifications).unwrap();
                    black_box((requests.recv().unwrap(), received));
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
