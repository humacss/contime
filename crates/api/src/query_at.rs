use crossbeam_channel::{unbounded, Sender};

use crate::contime_api::{ApiError, SnapshotQueryOutput};

traitstub::deps! {
    send_query_at[T, S, Ids](time: T, snapshot_ids: Ids, response: crossbeam_channel::Sender<Vec<Box<S>>>)
        -> Result<(), source::contime_api::ApiError>
        where [Ids: IntoIterator<Item = u128>];
}

/// Sends a historical snapshot query and collects results until all
/// downstream response senders have closed.
///
/// Result batches are flattened in receive order without sorting or adding
/// placeholders for missing snapshots. Empty IDs return an empty result.
///
/// # Errors
///
/// Returns [`ApiError::OutputChannelClosed`] if forwarding fails.
pub fn query_at<O, T, S, Ids>(output: &Sender<O>, time: T, snapshot_ids: Ids) -> Result<Vec<Box<S>>, ApiError>
where
    O: SnapshotQueryOutput<T, S>,
    Ids: IntoIterator<Item = u128>,
{
    query_at_with_deps(
        &Deps(|time, snapshot_ids, response| crate::contime_api::send_query_at(output, time, snapshot_ids, response)),
        time,
        snapshot_ids,
    )
}

fn query_at_with_deps<T, S, Ids>(deps: &impl calls::send_query_at<T, S, Ids>, time: T, snapshot_ids: Ids) -> Result<Vec<Box<S>>, ApiError>
where
    Ids: IntoIterator<Item = u128>,
{
    let (response, results) = unbounded();
    deps.send_query_at(time, snapshot_ids, response)?;
    Ok(results.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    traitstub::stubs! {
        send_query_at => |_, _, _| Ok(());
    }

    #[test]
    fn collects_every_snapshot_batch_until_sender_closure() {
        let received = RefCell::new(Vec::new());
        let responses = [vec![3_u64], vec![5, 8]];
        let deps = traitstub::defs! {
            send_query_at => |time, snapshot_ids: [u128; 2], response: Sender<Vec<Box<u64>>>| {
                received.borrow_mut().push((time, snapshot_ids.into_iter().collect::<Vec<_>>()));
                for snapshots in &responses {
                    response.send(snapshots.iter().copied().map(Box::new).collect()).unwrap();
                }
                Ok(())
            };
        };

        let result = query_at_with_deps(&deps, 42_u64, [7, 9]).unwrap();

        assert_eq!(*received.borrow(), vec![(42, vec![7, 9])]);
        assert_eq!(result.into_iter().map(|value| *value).collect::<Vec<_>>(), vec![3, 5, 8]);
    }

    #[test]
    fn propagates_enqueue_errors() {
        let deps = traitstub::defs! {
            send_query_at => |_: u64, _: [u128; 1], _: Sender<Vec<Box<u64>>>| Err(ApiError::OutputChannelClosed);
        };
        assert_eq!(query_at_with_deps(&deps, 42, [7]), Err(ApiError::OutputChannelClosed));
    }

    #[test]
    fn shared_stubs_close_the_response_channel_without_results() {
        assert_eq!(query_at_with_deps::<u64, u64, _>(&Stubs, 42, [7]), Ok(Vec::new()));
    }

    struct QueryOutput {
        time: u64,
        snapshot_ids: Vec<u128>,
        #[allow(clippy::vec_box)]
        response: Sender<Vec<Box<u64>>>,
    }

    impl SnapshotQueryOutput<u64, u64> for QueryOutput {
        fn snapshot_query(time: u64, snapshot_ids: Vec<u128>, response: Sender<Vec<Box<u64>>>) -> Self {
            Self { time, snapshot_ids, response }
        }
    }

    #[test]
    fn public_query_binds_the_real_sender_and_accepts_borrowed_iterators() {
        let (output, requests) = unbounded::<QueryOutput>();
        let ids = [7, 9];
        std::thread::scope(|scope| {
            let downstream = scope.spawn(move || {
                let request = requests.recv().unwrap();
                assert_eq!((request.time, request.snapshot_ids), (42, vec![7, 9]));
                request.response.send(vec![Box::new(3)]).unwrap();
                request.response.send(vec![Box::new(5), Box::new(8)]).unwrap();
            });
            let result = query_at(&output, 42, ids.iter().copied()).unwrap();
            assert_eq!(result.into_iter().map(|value| *value).collect::<Vec<_>>(), vec![3, 5, 8]);
            downstream.join().unwrap();
        });
    }

    #[test]
    fn public_query_preserves_empty_ids_and_closed_output_behavior() {
        let (output, requests) = unbounded::<QueryOutput>();
        drop(requests);
        assert_eq!(query_at(&output, 42, []), Ok(Vec::new()));
        assert_eq!(query_at(&output, 42, [7]), Err(ApiError::OutputChannelClosed));
    }
}
