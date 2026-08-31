use crossbeam_channel::{unbounded, Sender};

use crate::{ApiError, SnapshotQueryOutput};

trait Deps<O, T, S> {
    type Error;

    fn send_query_at<Ids>(&self, time: T, snapshot_ids: Ids, response: Sender<Vec<Box<S>>>) -> Result<(), Self::Error>
    where
        Ids: IntoIterator<Item = u128>;
}

struct DefaultDeps<'a, O> {
    output: &'a Sender<O>,
}

impl<O, T, S> Deps<O, T, S> for DefaultDeps<'_, O>
where
    O: SnapshotQueryOutput<T, S>,
{
    type Error = ApiError;

    fn send_query_at<Ids>(&self, time: T, snapshot_ids: Ids, response: Sender<Vec<Box<S>>>) -> Result<(), Self::Error>
    where
        Ids: IntoIterator<Item = u128>,
    {
        crate::send_query_at(self.output, time, snapshot_ids, response)
    }
}

/// Sends a historical snapshot query and collects results until all
/// downstream response senders have closed.
pub fn query_at<O, T, S, Ids>(output: &Sender<O>, time: T, snapshot_ids: Ids) -> Result<Vec<Box<S>>, ApiError>
where
    O: SnapshotQueryOutput<T, S>,
    Ids: IntoIterator<Item = u128>,
{
    query_at_with_deps(&DefaultDeps { output }, time, snapshot_ids)
}

fn query_at_with_deps<D, O, T, S, Ids>(deps: &D, time: T, snapshot_ids: Ids) -> Result<Vec<Box<S>>, D::Error>
where
    D: Deps<O, T, S>,
    Ids: IntoIterator<Item = u128>,
{
    let (response, results) = unbounded();
    deps.send_query_at(time, snapshot_ids, response)?;
    Ok(results.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crossbeam_channel::Sender;

    use super::{query_at_with_deps, Deps};

    #[derive(Debug, Eq, PartialEq)]
    struct StubError;

    struct StubDeps {
        received: RefCell<Vec<(u64, Vec<u128>)>>,
        responses: Vec<Vec<Box<u64>>>,
    }

    impl Deps<(), u64, u64> for StubDeps {
        type Error = StubError;

        fn send_query_at<Ids>(&self, time: u64, snapshot_ids: Ids, response: Sender<Vec<Box<u64>>>) -> Result<(), Self::Error>
        where
            Ids: IntoIterator<Item = u128>,
        {
            self.received.borrow_mut().push((time, snapshot_ids.into_iter().collect()));
            for snapshots in &self.responses {
                response.send(snapshots.iter().map(|snapshot| Box::new(**snapshot)).collect()).unwrap();
            }
            Ok(())
        }
    }

    struct FailingDeps;

    impl Deps<(), u64, u64> for FailingDeps {
        type Error = StubError;

        fn send_query_at<Ids>(&self, _time: u64, _snapshot_ids: Ids, _response: Sender<Vec<Box<u64>>>) -> Result<(), Self::Error>
        where
            Ids: IntoIterator<Item = u128>,
        {
            Err(StubError)
        }
    }

    #[test]
    fn collects_every_snapshot_batch_until_sender_closure() {
        let deps = StubDeps { received: RefCell::new(Vec::new()), responses: vec![vec![Box::new(3)], vec![Box::new(5), Box::new(8)]] };

        let result = query_at_with_deps(&deps, 42, [7, 9]).unwrap();

        assert_eq!(*deps.received.borrow(), vec![(42, vec![7, 9])]);
        assert_eq!(result.into_iter().map(|value| *value).collect::<Vec<_>>(), vec![3, 5, 8]);
    }

    #[test]
    fn propagates_enqueue_errors() {
        assert_eq!(query_at_with_deps(&FailingDeps, 42, [7]), Err(StubError));
    }
}
