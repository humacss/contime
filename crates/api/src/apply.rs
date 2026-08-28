use std::sync::Arc;

use contime::EventRejection;
use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::types::{ApiError, ApplyResponse, InputBatch, RejectionMessage};

trait Deps<E> {
    type Error;

    fn send<IL, I>(&self, inputs: I, rejection_sender: Sender<RejectionMessage>) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = IL>,
        IL: Into<Arc<E>>;
}

struct DefaultDeps<'a, IL> {
    output: &'a Sender<InputBatch<IL>>,
}

impl<IL> Deps<IL> for DefaultDeps<'_, IL> {
    type Error = ApiError;

    fn send<Input, I>(&self, inputs: I, rejection_sender: Sender<RejectionMessage>) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Input>,
        Input: Into<Arc<IL>>,
    {
        crate::send::send(self.output, inputs, rejection_sender)
    }
}

/// Sends inputs and waits until all downstream copies have either finished or
/// reported a rejection.
pub fn apply<E, IL, I>(output: &Sender<InputBatch<E>>, inputs: I) -> Result<ApplyResponse, ApiError>
where
    I: IntoIterator<Item = IL>,
    IL: Into<Arc<E>>,
{
    apply_with_deps(&DefaultDeps { output }, inputs)
}

fn apply_with_deps<D, E, IL, I>(deps: &D, inputs: I) -> Result<ApplyResponse, D::Error>
where
    D: Deps<E>,
    I: IntoIterator<Item = IL>,
    IL: Into<Arc<E>>,
{
    let (rejection_sender, rejection_receiver) = unbounded();
    deps.send(inputs, rejection_sender)?;
    Ok(collect_rejections(rejection_receiver))
}

fn collect_rejections(rejections: Receiver<RejectionMessage>) -> ApplyResponse {
    let mut rejections = rejections.into_iter().map(|message| EventRejection::new(message.event_id, message.reason)).collect::<Vec<_>>();
    rejections.sort_unstable();
    rejections.dedup();
    rejections
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::hint::black_box;
    use std::sync::Arc;

    use contime::{EventRejection, EventRejectionReason};
    use criterion::Criterion;
    use crossbeam_channel::Sender;

    use super::{apply_with_deps, Deps};
    use crate::types::RejectionMessage;

    #[derive(Debug, PartialEq, Eq)]
    struct StubError;

    struct StubDeps {
        received_inputs: RefCell<Vec<u128>>,
        responses: Vec<RejectionMessage>,
    }

    impl Deps<u128> for StubDeps {
        type Error = StubError;

        fn send<IL, I>(&self, inputs: I, rejection_sender: Sender<RejectionMessage>) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = IL>,
            IL: Into<Arc<u128>>,
        {
            self.received_inputs.borrow_mut().extend(inputs.into_iter().map(|input| *input.into()));
            for response in &self.responses {
                rejection_sender.send(response.clone()).unwrap();
            }
            Ok(())
        }
    }

    struct FailingDeps;

    impl Deps<u128> for FailingDeps {
        type Error = StubError;

        fn send<IL, I>(&self, _inputs: I, _rejection_sender: Sender<RejectionMessage>) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = IL>,
            IL: Into<Arc<u128>>,
        {
            Err(StubError)
        }
    }

    struct NoopDeps;

    impl Deps<()> for NoopDeps {
        type Error = StubError;

        fn send<IL, I>(&self, _inputs: I, _rejection_sender: Sender<RejectionMessage>) -> Result<(), Self::Error>
        where
            I: IntoIterator<Item = IL>,
            IL: Into<Arc<()>>,
        {
            Ok(())
        }
    }

    #[test]
    fn apply_forwards_inputs_to_send() {
        let deps = StubDeps { received_inputs: RefCell::new(Vec::new()), responses: Vec::new() };

        let result = apply_with_deps(&deps, [3, 5, 8]).unwrap();

        assert!(result.is_empty());
        assert_eq!(*deps.received_inputs.borrow(), vec![3, 5, 8]);
    }

    #[test]
    fn apply_sorts_and_deduplicates_rejections() {
        let duplicate = RejectionMessage { event_id: 7, reason: EventRejectionReason::MemoryFull };
        let earlier = RejectionMessage { event_id: 3, reason: EventRejectionReason::BeforeHistoryHorizon };
        let deps =
            StubDeps { received_inputs: RefCell::new(Vec::new()), responses: vec![duplicate.clone(), duplicate.clone(), earlier.clone()] };

        let result = apply_with_deps(&deps, [11]).unwrap();

        assert_eq!(
            result,
            vec![EventRejection::new(earlier.event_id, earlier.reason), EventRejection::new(duplicate.event_id, duplicate.reason),]
        );
    }

    #[test]
    fn apply_returns_an_empty_response_when_send_reports_no_rejections() {
        let deps = StubDeps { received_inputs: RefCell::new(Vec::new()), responses: Vec::new() };

        let result = apply_with_deps(&deps, [13]).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn apply_propagates_send_errors() {
        let result = apply_with_deps(&FailingDeps, [13]);

        assert_eq!(result, Err(StubError));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_apply() {
        let mut criterion = Criterion::default();

        criterion.bench_function("api/apply/no_rejections", |bencher| {
            bencher.iter(|| black_box(apply_with_deps(&NoopDeps, [()]).unwrap()));
        });

        criterion.final_summary();
    }
}
