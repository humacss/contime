use crossbeam_channel::{unbounded, Sender};

use crate::contime_api::{ApiError, EventQueryOutput};

traitstub::deps! {
    send_query_events_between[T, E](snapshot_id: u128, from: T, to: T, response: crossbeam_channel::Sender<Vec<E>>)
        -> Result<(), source::contime_api::ApiError>;
}

/// Sends an event-history query and collects results until all downstream
/// response senders have closed.
///
/// Queries the half-open interval `[from, to)` and flattens result batches in
/// receive order. This helper does not validate bounds, sort or deduplicate.
///
/// # Errors
///
/// Returns [`ApiError::OutputChannelClosed`] if forwarding fails.
pub fn query_events_between<O, T, E>(output: &Sender<O>, snapshot_id: u128, from: T, to: T) -> Result<Vec<E>, ApiError>
where
    O: EventQueryOutput<T, E>,
{
    query_events_between_with_deps(
        &Deps(|snapshot_id, from, to, response| crate::contime_api::send_query_events_between(output, snapshot_id, from, to, response)),
        snapshot_id,
        from,
        to,
    )
}

fn query_events_between_with_deps<T, E>(
    deps: &impl calls::send_query_events_between<T, E>,
    snapshot_id: u128,
    from: T,
    to: T,
) -> Result<Vec<E>, ApiError> {
    let (response, results) = unbounded();
    deps.send_query_events_between(snapshot_id, from, to, response)?;
    Ok(results.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    traitstub::stubs! {
        send_query_events_between => |_, _, _, _| Ok(());
    }

    #[test]
    fn shared_defaults_return_no_events() {
        assert_eq!(query_events_between_with_deps::<u64, u64>(&Stubs, 7, 10, 30), Ok(Vec::new()));
    }

    #[test]
    fn collects_event_batches_until_sender_closure() {
        let responses = [vec![1, 2], vec![3]];
        let deps = traitstub::defs! {
            send_query_events_between => |snapshot_id, from, to, response: Sender<Vec<u64>>| {
                assert_eq!((snapshot_id, from, to), (7, 10_u64, 30_u64));
                for events in &responses {
                    response.send(events.clone()).unwrap();
                }
                Ok(())
            };
        };

        let result = query_events_between_with_deps(&deps, 7, 10, 30).unwrap();

        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn propagates_enqueue_errors() {
        let deps = traitstub::defs! {
            send_query_events_between => |_, _: u64, _: u64, _: Sender<Vec<u64>>| Err(ApiError::OutputChannelClosed);
        };
        assert_eq!(query_events_between_with_deps(&deps, 7, 10, 30), Err(ApiError::OutputChannelClosed));
    }

    struct QueryOutput {
        snapshot_id: u128,
        from: String,
        to: String,
        response: Sender<Vec<String>>,
    }

    impl EventQueryOutput<String, String> for QueryOutput {
        fn event_query(snapshot_id: u128, from: String, to: String, response: Sender<Vec<String>>) -> Self {
            Self { snapshot_id, from, to, response }
        }
    }

    #[test]
    fn public_query_preserves_bounds_batch_order_and_duplicates() {
        let (output, requests) = unbounded::<QueryOutput>();
        std::thread::scope(|scope| {
            let downstream = scope.spawn(move || {
                let request = requests.recv().unwrap();
                assert_eq!((request.snapshot_id, request.from.as_str(), request.to.as_str()), (7, "z", "a"));
                request.response.send(vec!["third".into(), "first".into()]).unwrap();
                request.response.send(Vec::new()).unwrap();
                request.response.send(vec!["first".into(), "second".into()]).unwrap();
            });
            let result = query_events_between(&output, 7, "z".into(), "a".into()).unwrap();
            assert_eq!(result, vec!["third", "first", "first", "second"]);
            downstream.join().unwrap();
        });
    }

    #[test]
    fn public_query_reports_a_closed_output_channel() {
        let (output, requests) = unbounded::<QueryOutput>();
        drop(requests);
        assert_eq!(query_events_between(&output, 7, "a".into(), "z".into()), Err(ApiError::OutputChannelClosed));
    }
}
