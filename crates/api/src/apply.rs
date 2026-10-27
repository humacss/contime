use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::types::{ApiError, ApplyOutput, ApplyResponse, RejectionMessage};

traitstub::deps! {
    send[R, Inputs](inputs: Inputs, rejection_sender: crossbeam_channel::Sender<source::RejectionMessage<R>>)
        -> Result<(), source::ApiError>
        where [Inputs: IntoIterator];
}

/// Sends inputs and collects rejections until all downstream request handles close.
/// Whether closure includes replay is defined by the downstream adapter.
/// Results are sorted by event ID and reason, with identical pairs deduplicated.
/// Empty input returns an empty response without queueing work.
///
/// # Errors
///
/// Returns [`ApiError::OutputChannelClosed`] if forwarding fails. Rejected inputs
/// are returned in the successful response rather than as this error.
pub fn apply<O, I, R, Input, Inputs>(output: &Sender<O>, inputs: Inputs) -> Result<ApplyResponse<R>, ApiError>
where
    Inputs: IntoIterator<Item = Input>,
    Input: Into<I>,
    O: ApplyOutput<I, R>,
    R: Ord,
{
    apply_with_deps(&Deps(|inputs, rejection_sender| crate::send::send::<O, I, R, Input, Inputs>(output, inputs, rejection_sender)), inputs)
}

fn apply_with_deps<R, Inputs>(deps: &impl calls::send<R, Inputs>, inputs: Inputs) -> Result<ApplyResponse<R>, ApiError>
where
    Inputs: IntoIterator,
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
    use std::cell::RefCell;
    use std::hint::black_box;

    use super::*;
    use crate::types::InputBatch;

    #[derive(Clone, Copy, Debug, Ord, PartialOrd, Eq, PartialEq)]
    enum TestReason {
        BeforeHistoryHorizon,
        MemoryFull,
    }

    traitstub::stubs! {
        send => |_, _| Ok(());
    }

    #[test]
    fn apply_forwards_inputs_to_send() {
        let received_inputs = RefCell::new(Vec::new());
        let deps = traitstub::defs! {
            send => |inputs: [u128; 3], _: Sender<RejectionMessage<TestReason>>| {
                received_inputs.borrow_mut().extend(inputs);
                Ok(())
            };
        };

        let result = apply_with_deps(&deps, [3_u128, 5, 8]).unwrap();

        assert!(result.is_empty());
        assert_eq!(*received_inputs.borrow(), vec![3, 5, 8]);
    }

    #[test]
    fn apply_sorts_and_deduplicates_rejections() {
        let duplicate = RejectionMessage { event_id: 7, reason: TestReason::MemoryFull };
        let earlier = RejectionMessage { event_id: 3, reason: TestReason::BeforeHistoryHorizon };
        let same_event = RejectionMessage { event_id: 7, reason: TestReason::BeforeHistoryHorizon };
        let responses = [duplicate.clone(), duplicate.clone(), same_event.clone(), earlier.clone()];
        let deps = traitstub::defs! {
            send => |_: [u128; 1], rejection_sender: Sender<RejectionMessage<TestReason>>| {
                for response in &responses {
                    rejection_sender.send(response.clone()).unwrap();
                }
                Ok(())
            };
        };

        let result = apply_with_deps(&deps, [11_u128]).unwrap();

        assert_eq!(result, vec![earlier, same_event, duplicate]);
    }

    #[test]
    fn apply_returns_an_empty_response_when_send_reports_no_rejections() {
        let result = apply_with_deps::<TestReason, _>(&Stubs, [13_u128]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn apply_propagates_send_errors() {
        let deps = traitstub::defs! {
            send => |_: [u128; 1], _: Sender<RejectionMessage<TestReason>>| Err(ApiError::OutputChannelClosed);
        };
        assert_eq!(apply_with_deps(&deps, [13_u128]), Err(ApiError::OutputChannelClosed));
    }

    #[test]
    fn public_apply_converts_inputs_and_collects_real_rejections() {
        let (output, requests) = unbounded::<InputBatch<u128, TestReason>>();
        let inputs = [3_u64, 5, 8];
        std::thread::scope(|scope| {
            let downstream = scope.spawn(move || {
                let request = requests.recv().unwrap();
                assert_eq!(request.inputs, vec![3_u128, 5, 8]);
                for event_id in [8, 3, 8] {
                    request.rejection_sender.send(RejectionMessage { event_id, reason: TestReason::MemoryFull }).unwrap();
                }
            });
            let result = apply(&output, inputs.iter().copied()).unwrap();
            assert_eq!(
                result,
                vec![
                    RejectionMessage { event_id: 3, reason: TestReason::MemoryFull },
                    RejectionMessage { event_id: 8, reason: TestReason::MemoryFull },
                ]
            );
            downstream.join().unwrap();
        });
    }

    #[test]
    fn public_apply_preserves_empty_input_and_closed_output_behavior() {
        let (output, requests) = unbounded::<InputBatch<u128, TestReason>>();
        drop(requests);
        assert_eq!(apply(&output, [] as [u128; 0]), Ok(Vec::new()));
        assert_eq!(apply(&output, [3_u128]), Err(ApiError::OutputChannelClosed));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_apply() {
        let mut criterion = Criterion::default();

        criterion.bench_function("api/apply/no_rejections", |bencher| {
            bencher.iter(|| black_box(apply_with_deps::<TestReason, _>(&Stubs, [()]).unwrap()));
        });

        criterion.final_summary();
    }
}
