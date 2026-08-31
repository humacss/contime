use crossbeam_channel::{unbounded, Sender};

use crate::{ApiError, EventQueryOutput};

trait Deps<O, T, E> {
    type Error;

    fn send_query_events_between(&self, snapshot_id: u128, from: T, to: T, response: Sender<Vec<E>>) -> Result<(), Self::Error>;
}

struct DefaultDeps<'a, O> {
    output: &'a Sender<O>,
}

impl<O, T, E> Deps<O, T, E> for DefaultDeps<'_, O>
where
    O: EventQueryOutput<T, E>,
{
    type Error = ApiError;

    fn send_query_events_between(&self, snapshot_id: u128, from: T, to: T, response: Sender<Vec<E>>) -> Result<(), Self::Error> {
        crate::send_query_events_between(self.output, snapshot_id, from, to, response)
    }
}

/// Sends an event-history query and collects results until all downstream
/// response senders have closed.
pub fn query_events_between<O, T, E>(output: &Sender<O>, snapshot_id: u128, from: T, to: T) -> Result<Vec<E>, ApiError>
where
    O: EventQueryOutput<T, E>,
{
    query_events_between_with_deps(&DefaultDeps { output }, snapshot_id, from, to)
}

fn query_events_between_with_deps<D, O, T, E>(deps: &D, snapshot_id: u128, from: T, to: T) -> Result<Vec<E>, D::Error>
where
    D: Deps<O, T, E>,
{
    let (response, results) = unbounded();
    deps.send_query_events_between(snapshot_id, from, to, response)?;
    Ok(results.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::Sender;

    use super::{query_events_between_with_deps, Deps};

    #[derive(Debug, Eq, PartialEq)]
    struct StubError;

    struct StubDeps {
        responses: Vec<Vec<u64>>,
    }

    impl Deps<(), u64, u64> for StubDeps {
        type Error = StubError;

        fn send_query_events_between(&self, snapshot_id: u128, from: u64, to: u64, response: Sender<Vec<u64>>) -> Result<(), Self::Error> {
            assert_eq!((snapshot_id, from, to), (7, 10, 30));
            for events in &self.responses {
                response.send(events.clone()).unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn collects_event_batches_until_sender_closure() {
        let deps = StubDeps { responses: vec![vec![1, 2], vec![3]] };

        let result = query_events_between_with_deps(&deps, 7, 10, 30).unwrap();

        assert_eq!(result, vec![1, 2, 3]);
    }
}
