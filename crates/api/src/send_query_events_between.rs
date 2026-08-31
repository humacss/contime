use crossbeam_channel::Sender;

use crate::{ApiError, EventQueryOutput};

/// Forwards one asynchronous half-open event-history query.
pub fn send_query_events_between<O, T, E>(
    output: &Sender<O>,
    snapshot_id: u128,
    from: T,
    to: T,
    response: Sender<Vec<E>>,
) -> Result<(), ApiError>
where
    O: EventQueryOutput<T, E>,
{
    output.send(O::event_query(snapshot_id, from, to, response)).map_err(|_| ApiError::OutputChannelClosed)
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::{unbounded, Sender};

    use crate::{send_query_events_between, ApiError, EventQueryOutput};

    struct QueryOutput {
        snapshot_id: u128,
        from: u64,
        to: u64,
        response: Sender<Vec<u64>>,
    }

    impl EventQueryOutput<u64, u64> for QueryOutput {
        fn event_query(snapshot_id: u128, from: u64, to: u64, response: Sender<Vec<u64>>) -> Self {
            Self { snapshot_id, from, to, response }
        }
    }

    #[test]
    fn forwards_one_event_query_without_interpreting_bounds() {
        let (output, receiver) = unbounded::<QueryOutput>();
        let (response, results) = unbounded();

        send_query_events_between(&output, 7, 30, 10, response).unwrap();

        let query = receiver.recv().unwrap();
        assert_eq!((query.snapshot_id, query.from, query.to), (7, 30, 10));
        query.response.send(vec![3, 5]).unwrap();
        assert_eq!(results.recv().unwrap(), vec![3, 5]);
    }

    #[test]
    fn reports_a_closed_output_channel() {
        let (output, receiver) = unbounded::<QueryOutput>();
        let (response, _results) = unbounded();
        drop(receiver);

        assert_eq!(send_query_events_between(&output, 7, 10, 30, response), Err(ApiError::OutputChannelClosed));
    }
}
