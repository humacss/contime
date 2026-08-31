use crossbeam_channel::Sender;

use crate::{ApiError, SnapshotQueryOutput};

/// Forwards one asynchronous historical snapshot query.
pub fn send_query_at<O, T, S, Ids>(output: &Sender<O>, time: T, snapshot_ids: Ids, response: Sender<Vec<Box<S>>>) -> Result<(), ApiError>
where
    O: SnapshotQueryOutput<T, S>,
    Ids: IntoIterator<Item = u128>,
{
    let snapshot_ids = snapshot_ids.into_iter().collect::<Vec<_>>();
    if snapshot_ids.is_empty() {
        return Ok(());
    }

    output.send(O::snapshot_query(time, snapshot_ids, response)).map_err(|_| ApiError::OutputChannelClosed)
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::{unbounded, Sender, TryRecvError};

    use crate::{send_query_at, ApiError, SnapshotQueryOutput};

    struct QueryOutput {
        time: u64,
        snapshot_ids: Vec<u128>,
        response: Sender<Vec<Box<u64>>>,
    }

    impl SnapshotQueryOutput<u64, u64> for QueryOutput {
        fn snapshot_query(time: u64, snapshot_ids: Vec<u128>, response: Sender<Vec<Box<u64>>>) -> Self {
            Self { time, snapshot_ids, response }
        }
    }

    #[test]
    fn forwards_one_snapshot_query_with_its_response_sender() {
        let (output, receiver) = unbounded::<QueryOutput>();
        let (response, results) = unbounded();

        send_query_at(&output, 42, [3, 5, 8], response).unwrap();

        let query = receiver.recv().unwrap();
        assert_eq!(query.time, 42);
        assert_eq!(query.snapshot_ids, vec![3, 5, 8]);
        query.response.send(vec![Box::new(13)]).unwrap();
        assert_eq!(*results.recv().unwrap()[0], 13);
    }

    #[test]
    fn empty_snapshot_queries_close_without_forwarding() {
        let (output, receiver) = unbounded::<QueryOutput>();
        let (response, results) = unbounded();

        send_query_at(&output, 42, [] as [u128; 0], response).unwrap();

        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(results.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn reports_a_closed_output_channel() {
        let (output, receiver) = unbounded::<QueryOutput>();
        let (response, _results) = unbounded();
        drop(receiver);

        assert_eq!(send_query_at(&output, 42, [3], response), Err(ApiError::OutputChannelClosed));
    }
}
