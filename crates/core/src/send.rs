use crossbeam_channel::Sender;

use crate::input::prepare_inputs;
use crate::{ApiError, ConTime, Input, RejectionMessage, RejectionReason, RouterBatch};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    pub fn send(
        &self,
        inputs: impl IntoIterator<Item = I>,
        rejection_sender: Sender<RejectionMessage<RejectionReason>>,
    ) -> Result<(), ApiError> {
        send_to(&self.budget, self.runtime.input(), inputs, rejection_sender)
    }
}

fn send_to<I>(
    budget: &crate::MemoryBudget,
    output: &Sender<RouterBatch<I>>,
    inputs: impl IntoIterator<Item = I>,
    rejection_sender: Sender<RejectionMessage<RejectionReason>>,
) -> Result<(), ApiError>
where
    I: Input,
{
    let inputs = inputs.into_iter().collect::<Vec<_>>();
    let inputs = match prepare_inputs(budget, inputs) {
        Ok(inputs) => inputs,
        Err(rejections) => {
            for rejection in rejections {
                let _ = rejection_sender.send(rejection);
            }
            return Ok(());
        }
    };
    contime_api::send::<RouterBatch<I>, _, _, _, _>(output, inputs, rejection_sender)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use contime_checkpoints::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, CheckpointConfig, EventBatch, Snapshot};
    use contime_memory::ConservativeTrackedSize;
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::unbounded;

    use super::send_to;
    use crate::{ConTime, ConTimeConfig, Input, MemoryBudget, RejectionMessage, RejectionReason, RouterBatch};

    struct TestInput {
        id: u128,
        value: usize,
    }

    impl ConservativeTrackedSize for TestInput {
        fn conservative_tracked_size(&self) -> usize {
            64
        }
    }

    impl Input for TestInput {
        type Time = i64;

        fn event_id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> Self::Time {
            1
        }

        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }

    #[derive(Clone, Default)]
    struct TestSnapshot {
        value: usize,
    }

    impl ConservativeTrackedSize for TestSnapshot {
        fn conservative_tracked_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    impl Snapshot for TestSnapshot {
        type Time = i64;

        fn set_time(&mut self, _time: Self::Time) {}
    }

    impl ApplyEvents<TestInput> for TestSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestInput) -> Self {
            Self::default()
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, TestInput>) {
            self.value += batch.events.iter().map(|event| event.value).sum::<usize>();
        }
    }

    #[derive(Clone)]
    struct RecordingWrapper(Arc<AtomicUsize>);

    impl ApplyWrapper<TestSnapshot, TestInput> for RecordingWrapper {
        fn apply_event_batch(&mut self, batch: EventBatch<'_, i64, TestInput>, apply_inner: &mut ApplyInner<'_, TestSnapshot>) {
            apply_inner.apply_event_batch(batch);
            self.0.store(apply_inner.snapshot().value, Ordering::Relaxed);
        }
    }

    fn config(memory_limit: usize, memory_buffer: usize) -> ConTimeConfig {
        ConTimeConfig {
            router_count: 1,
            worker_count: 1,
            router_seed: 9,
            memory_limit,
            memory_buffer,
            worker: contime_worker::WorkerConfig {
                maximum_dirty_age: Duration::from_micros(100),
                replays_per_receive: 1,
                deadline_compaction_minimum: 1_024,
                deadline_compaction_multiplier: 2,
            },
            checkpoints: CheckpointConfig { interval: 100 },
        }
    }

    #[test]
    fn one_receiver_closure_reports_that_every_sent_batch_was_applied() {
        let observed = Arc::new(AtomicUsize::new(0));
        let contime =
            ConTime::<TestInput, TestSnapshot, RecordingWrapper>::start(config(100_000, 1_000), RecordingWrapper(Arc::clone(&observed)))
                .unwrap();
        let (sender, receiver) = unbounded::<RejectionMessage<RejectionReason>>();

        contime.send([TestInput { id: 1, value: 5 }], sender.clone()).unwrap();
        contime.send([TestInput { id: 2, value: 7 }], sender.clone()).unwrap();
        drop(sender);

        assert_eq!(receiver.into_iter().collect::<Vec<_>>(), Vec::new());
        assert_eq!(observed.load(Ordering::Relaxed), 12);
        contime.shutdown();
    }

    #[test]
    fn rejected_send_reports_every_input_through_the_supplied_channel() {
        let observed = Arc::new(AtomicUsize::new(0));
        let contime =
            ConTime::<TestInput, TestSnapshot, RecordingWrapper>::start(config(100, 50), RecordingWrapper(Arc::clone(&observed))).unwrap();
        let (sender, receiver) = unbounded::<RejectionMessage<RejectionReason>>();

        contime.send([TestInput { id: 1, value: 5 }, TestInput { id: 2, value: 7 }], sender).unwrap();

        let rejections = receiver.into_iter().collect::<Vec<_>>();
        assert_eq!(rejections.len(), 2);
        assert!(rejections.iter().all(|rejection| rejection.reason == RejectionReason::MemoryFull));
        assert_eq!(observed.load(Ordering::Relaxed), 0);
        contime.shutdown();
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_send() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/send/prepare_and_forward_1000", |bencher| {
            bencher.iter_batched(
                || {
                    let budget = MemoryBudget::new(usize::MAX, 0);
                    let inputs = (0..1_000).map(|id| TestInput { id, value: 1 }).collect::<Vec<_>>();
                    let (rejection_sender, rejection_receiver) = unbounded();
                    let (output, output_receiver) = unbounded::<RouterBatch<TestInput>>();
                    (budget, inputs, rejection_sender, rejection_receiver, output, output_receiver)
                },
                |(budget, inputs, rejection_sender, rejection_receiver, output, output_receiver)| {
                    send_to(&budget, &output, inputs, rejection_sender).unwrap();
                    std::hint::black_box((budget, rejection_receiver, output, output_receiver))
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
