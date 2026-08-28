use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::types::{ApiError, InputBatch, RejectionMessage};

trait Deps<IL> {
    type Error;

    fn forward(&self, batch: InputBatch<IL>) -> Result<(), Self::Error>;
}

struct DefaultDeps<'a, IL> {
    output: &'a Sender<InputBatch<IL>>,
}

impl<IL> Deps<IL> for DefaultDeps<'_, IL> {
    type Error = ApiError;

    fn forward(&self, batch: InputBatch<IL>) -> Result<(), Self::Error> {
        self.output.send(batch).map_err(|_| ApiError::OutputChannelClosed)
    }
}

/// Normalizes owned or shared inputs into Arcs and forwards one batch.
///
/// The batch owns the rejection sender, allowing channel closure to signal
/// that every downstream copy of the batch has finished processing. Existing
/// Arcs preserve their allocation; owned values receive one Arc allocation.
pub fn send<E, IL, I>(output: &Sender<InputBatch<E>>, inputs: I, rejection_sender: Sender<RejectionMessage>) -> Result<(), ApiError>
where
    I: IntoIterator<Item = IL>,
    IL: Into<Arc<E>>,
{
    send_with_deps(&DefaultDeps { output }, inputs, rejection_sender)
}

fn send_with_deps<D, E, IL, I>(deps: &D, inputs: I, rejection_sender: Sender<RejectionMessage>) -> Result<(), D::Error>
where
    D: Deps<E>,
    I: IntoIterator<Item = IL>,
    IL: Into<Arc<E>>,
{
    let inputs = inputs.into_iter().map(Into::into).collect::<Vec<_>>();
    if inputs.is_empty() {
        return Ok(());
    }

    deps.forward(InputBatch { inputs, rejection_sender })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::hint::black_box;
    use std::sync::Arc;

    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, TryRecvError};

    use super::{send, send_with_deps, Deps};
    use crate::types::{ApiError, InputBatch};

    #[repr(C)]
    struct BenchmarkEvent<const PAYLOAD_BYTES: usize> {
        event_id: u128,
        payload: [u8; PAYLOAD_BYTES],
    }

    const _: () = assert!(size_of::<BenchmarkEvent<16>>() == 32);
    const _: () = assert!(size_of::<BenchmarkEvent<48>>() == 64);
    const _: () = assert!(size_of::<BenchmarkEvent<192>>() == 208);
    const _: () = assert!(size_of::<BenchmarkEvent<992>>() == 1_008);

    fn benchmark_event<const PAYLOAD_BYTES: usize>(input_index: usize) -> BenchmarkEvent<PAYLOAD_BYTES> {
        BenchmarkEvent { event_id: input_index as u128, payload: [input_index as u8; PAYLOAD_BYTES] }
    }

    fn benchmark_owned<const PAYLOAD_BYTES: usize>(criterion: &mut Criterion, event_bytes: usize, input_count: usize) {
        criterion.bench_function(&format!("api/send/owned/{event_bytes}_bytes/{input_count}_inputs"), |bencher| {
            bencher.iter_batched(
                || {
                    let inputs = (0..input_count).map(benchmark_event::<PAYLOAD_BYTES>).collect::<Vec<_>>();
                    let (rejection_sender, rejection_receiver) = unbounded();
                    let (output, output_receiver) = unbounded::<InputBatch<BenchmarkEvent<PAYLOAD_BYTES>>>();
                    (inputs, rejection_sender, rejection_receiver, output, output_receiver)
                },
                |(inputs, rejection_sender, rejection_receiver, output, output_receiver)| {
                    send(&output, inputs, rejection_sender).unwrap();
                    black_box((output_receiver, rejection_receiver))
                },
                BatchSize::LargeInput,
            );
        });
    }

    fn benchmark_shared<const PAYLOAD_BYTES: usize>(criterion: &mut Criterion, event_bytes: usize, input_count: usize) {
        criterion.bench_function(&format!("api/send/shared/{event_bytes}_bytes/{input_count}_inputs"), |bencher| {
            bencher.iter_batched(
                || {
                    let inputs = (0..input_count).map(benchmark_event::<PAYLOAD_BYTES>).map(Arc::new).collect::<Vec<_>>();
                    let (rejection_sender, rejection_receiver) = unbounded();
                    let (output, output_receiver) = unbounded::<InputBatch<BenchmarkEvent<PAYLOAD_BYTES>>>();
                    (inputs, rejection_sender, rejection_receiver, output, output_receiver)
                },
                |(inputs, rejection_sender, rejection_receiver, output, output_receiver)| {
                    send(&output, inputs, rejection_sender).unwrap();
                    black_box((output_receiver, rejection_receiver))
                },
                BatchSize::LargeInput,
            );
        });
    }

    #[derive(Debug, PartialEq, Eq)]
    struct StubError;

    struct StubDeps {
        batches: RefCell<Vec<InputBatch<u128>>>,
    }

    impl Deps<u128> for StubDeps {
        type Error = StubError;

        fn forward(&self, batch: InputBatch<u128>) -> Result<(), Self::Error> {
            self.batches.borrow_mut().push(batch);
            Ok(())
        }
    }

    struct FailingDeps;

    impl Deps<u128> for FailingDeps {
        type Error = StubError;

        fn forward(&self, _batch: InputBatch<u128>) -> Result<(), Self::Error> {
            Err(StubError)
        }
    }

    #[test]
    fn send_forwards_all_inputs_in_one_batch() {
        let deps = StubDeps { batches: RefCell::new(Vec::new()) };
        let (rejection_sender, _rejection_receiver) = unbounded();

        send_with_deps(&deps, [3, 5, 8], rejection_sender).unwrap();

        let batches = deps.batches.borrow();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].inputs.iter().map(|input| **input).collect::<Vec<_>>(), vec![3, 5, 8]);
    }

    #[test]
    fn rejection_channel_closes_after_the_input_batch_is_dropped() {
        let deps = StubDeps { batches: RefCell::new(Vec::new()) };
        let (rejection_sender, rejection_receiver) = unbounded();

        send_with_deps(&deps, [3, 5, 8], rejection_sender).unwrap();
        assert_eq!(rejection_receiver.try_recv(), Err(TryRecvError::Empty));

        deps.batches.borrow_mut().clear();
        assert_eq!(rejection_receiver.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn empty_send_drops_the_original_rejection_sender() {
        let deps = StubDeps { batches: RefCell::new(Vec::new()) };
        let (rejection_sender, rejection_receiver) = unbounded();

        send_with_deps(&deps, [] as [u128; 0], rejection_sender).unwrap();

        assert_eq!(rejection_receiver.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn send_propagates_dependency_errors() {
        let (rejection_sender, _rejection_receiver) = unbounded();

        let result = send_with_deps(&FailingDeps, [3], rejection_sender);

        assert_eq!(result, Err(StubError));
    }

    #[test]
    fn send_reports_a_closed_output_channel() {
        let (output, output_receiver) = unbounded();
        let (rejection_sender, _rejection_receiver) = unbounded();
        drop(output_receiver);

        let result = send(&output, [3], rejection_sender);

        assert_eq!(result, Err(ApiError::OutputChannelClosed));
    }

    #[test]
    fn send_wraps_owned_inputs_in_arcs() {
        let (output, output_receiver) = unbounded::<InputBatch<u128>>();
        let (rejection_sender, _rejection_receiver) = unbounded();

        send(&output, [3_u128], rejection_sender).unwrap();

        let batch = output_receiver.recv().unwrap();
        assert_eq!(*batch.inputs[0], 3);
    }

    #[test]
    fn send_preserves_existing_arc_allocations() {
        let input = Arc::new(5_u128);
        let input_pointer = Arc::as_ptr(&input);
        let (output, output_receiver) = unbounded::<InputBatch<u128>>();
        let (rejection_sender, _rejection_receiver) = unbounded();

        send(&output, [input], rejection_sender).unwrap();

        let batch = output_receiver.recv().unwrap();
        assert_eq!(Arc::as_ptr(&batch.inputs[0]), input_pointer);
        assert_eq!(Arc::strong_count(&batch.inputs[0]), 1);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_send() {
        let mut criterion = Criterion::default();

        for input_count in [1, 1_000] {
            benchmark_owned::<16>(&mut criterion, 32, input_count);
            benchmark_shared::<16>(&mut criterion, 32, input_count);
            benchmark_owned::<48>(&mut criterion, 64, input_count);
            benchmark_shared::<48>(&mut criterion, 64, input_count);
            benchmark_owned::<192>(&mut criterion, 208, input_count);
            benchmark_shared::<192>(&mut criterion, 208, input_count);
            benchmark_owned::<992>(&mut criterion, 1_008, input_count);
            benchmark_shared::<992>(&mut criterion, 1_008, input_count);
        }

        criterion.final_summary();
    }
}
