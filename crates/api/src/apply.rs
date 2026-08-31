use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::types::{ApiError, ApplyOutput, ApplyResponse, RejectionMessage};

trait Deps<O, I, R> {
    type Error;

    fn send<Input, Inputs>(&self, inputs: Inputs, rejection_sender: Sender<RejectionMessage<R>>) -> Result<(), Self::Error>
    where
        Inputs: IntoIterator<Item = Input>,
        Input: Into<I>;
}

struct DefaultDeps<'a, O> {
    output: &'a Sender<O>,
}

impl<O, I, R> Deps<O, I, R> for DefaultDeps<'_, O>
where
    O: ApplyOutput<I, R>,
{
    type Error = ApiError;

    fn send<Input, Inputs>(&self, inputs: Inputs, rejection_sender: Sender<RejectionMessage<R>>) -> Result<(), Self::Error>
    where
        Inputs: IntoIterator<Item = Input>,
        Input: Into<I>,
    {
        crate::send::send(self.output, inputs, rejection_sender)
    }
}

/// Sends inputs and waits until all downstream copies have either finished or
/// reported a rejection.
pub fn apply<O, I, R, Input, Inputs>(output: &Sender<O>, inputs: Inputs) -> Result<ApplyResponse<R>, ApiError>
where
    Inputs: IntoIterator<Item = Input>,
    Input: Into<I>,
    O: ApplyOutput<I, R>,
    R: Ord,
{
    apply_with_deps(&DefaultDeps { output }, inputs)
}

fn apply_with_deps<D, O, I, R, Input, Inputs>(deps: &D, inputs: Inputs) -> Result<ApplyResponse<R>, D::Error>
where
    D: Deps<O, I, R>,
    Inputs: IntoIterator<Item = Input>,
    Input: Into<I>,
    R: Ord,
{
    let (rejection_sender, rejection_receiver) = unbounded();
    deps.send(inputs, rejection_sender)?;
    Ok(collect_rejections(rejection_receiver))
}

fn collect_rejections<R>(rejections: Receiver<RejectionMessage<R>>) -> ApplyResponse<R>
where
    R: Ord,
{
    let mut rejections = rejections.into_iter().collect::<Vec<_>>();
    rejections.sort_unstable();
    rejections.dedup();
    rejections
}

#[cfg(test)]
mod tests {
    use criterion::Criterion;
    use crossbeam_channel::Sender;
    use std::cell::RefCell;
    use std::hint::black_box;

    use super::{apply_with_deps, Deps};
    use crate::types::RejectionMessage;

    #[derive(Debug, PartialEq, Eq)]
    struct StubError;

    #[derive(Clone, Copy, Debug, Ord, PartialOrd, Eq, PartialEq)]
    enum TestReason {
        BeforeHistoryHorizon,
        MemoryFull,
    }

    struct StubDeps {
        received_inputs: RefCell<Vec<u128>>,
        responses: Vec<RejectionMessage<TestReason>>,
    }

    impl Deps<(), u128, TestReason> for StubDeps {
        type Error = StubError;

        fn send<Input, Inputs>(&self, inputs: Inputs, rejection_sender: Sender<RejectionMessage<TestReason>>) -> Result<(), Self::Error>
        where
            Inputs: IntoIterator<Item = Input>,
            Input: Into<u128>,
        {
            self.received_inputs.borrow_mut().extend(inputs.into_iter().map(Into::into));
            for response in &self.responses {
                rejection_sender.send(response.clone()).unwrap();
            }
            Ok(())
        }
    }

    struct FailingDeps;

    impl Deps<(), u128, TestReason> for FailingDeps {
        type Error = StubError;

        fn send<Input, Inputs>(&self, _inputs: Inputs, _rejection_sender: Sender<RejectionMessage<TestReason>>) -> Result<(), Self::Error>
        where
            Inputs: IntoIterator<Item = Input>,
            Input: Into<u128>,
        {
            Err(StubError)
        }
    }

    struct NoopDeps;

    impl Deps<(), (), TestReason> for NoopDeps {
        type Error = StubError;

        fn send<Input, Inputs>(&self, _inputs: Inputs, _rejection_sender: Sender<RejectionMessage<TestReason>>) -> Result<(), Self::Error>
        where
            Inputs: IntoIterator<Item = Input>,
            Input: Into<()>,
        {
            Ok(())
        }
    }

    #[test]
    fn apply_forwards_inputs_to_send() {
        let deps = StubDeps { received_inputs: RefCell::new(Vec::new()), responses: Vec::new() };

        let result = apply_with_deps(&deps, [3_u128, 5, 8]).unwrap();

        assert!(result.is_empty());
        assert_eq!(*deps.received_inputs.borrow(), vec![3, 5, 8]);
    }

    #[test]
    fn apply_sorts_and_deduplicates_rejections() {
        let duplicate = RejectionMessage { event_id: 7, reason: TestReason::MemoryFull };
        let earlier = RejectionMessage { event_id: 3, reason: TestReason::BeforeHistoryHorizon };
        let deps =
            StubDeps { received_inputs: RefCell::new(Vec::new()), responses: vec![duplicate.clone(), duplicate.clone(), earlier.clone()] };

        let result = apply_with_deps(&deps, [11_u128]).unwrap();

        assert_eq!(result, vec![earlier, duplicate]);
    }

    #[test]
    fn apply_returns_an_empty_response_when_send_reports_no_rejections() {
        let deps = StubDeps { received_inputs: RefCell::new(Vec::new()), responses: Vec::new() };

        let result = apply_with_deps(&deps, [13_u128]).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn apply_propagates_send_errors() {
        let result = apply_with_deps(&FailingDeps, [13_u128]);

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
