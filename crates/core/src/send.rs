use crossbeam_channel::Sender;

use crate::input::prepare_inputs;
use crate::{ApiError, ConTime, Input, RejectionMessage, RejectionReason, RouterMessage};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    /// Low-level enqueue with a caller-owned rejection stream. Closure of this
    /// stream acknowledges insertion, not completion of scheduled replay.
    /// Prefer [`Self::apply`] with [`Self::errors`] for ordinary submissions.
    pub fn send(
        &self,
        inputs: impl IntoIterator<Item = I>,
        rejection_sender: Sender<RejectionMessage<RejectionReason>>,
    ) -> Result<(), ApiError> {
        send_to(&self.input, inputs, rejection_sender)
    }

    /// Submits causal output from an active application. Outputs must be
    /// enqueued before that application returns and cannot precede its time.
    /// Unlike external input, these records use the proven safe boundary.
    pub fn apply_internal(&self, source: I::Time, inputs: impl IntoIterator<Item = I>) -> Result<(), ApiError> {
        let inputs = prepare_inputs(inputs.into_iter().collect());
        self.input
            .send(RouterMessage::Internal {
                source,
                batch: crate::RouterBatch { inputs, completion: crate::CompletionHandle { sender: self.error_sender.clone() } },
            })
            .map_err(|_| ApiError::OutputChannelClosed)
    }
}

fn send_to<I, S>(
    output: &Sender<RouterMessage<I, S>>,
    inputs: impl IntoIterator<Item = I>,
    rejection_sender: Sender<RejectionMessage<RejectionReason>>,
) -> Result<(), ApiError>
where
    I: Input,
{
    let inputs = inputs.into_iter().collect::<Vec<_>>();
    let inputs = prepare_inputs(inputs);
    contime_api::send::<RouterMessage<I, S>, _, _, _, _>(output, inputs, rejection_sender)
}

#[cfg(test)]
mod tests {
    use crate::types::testing::Time;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::checkpoints::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, CheckpointConfig, EventBatch, Snapshot};

    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::unbounded;

    use super::send_to;
    use crate::{ConTime, ConTimeConfig, Input, RejectionMessage, RejectionReason, RouterMessage};

    struct TestInput {
        id: u128,
        value: usize,
    }

    impl contime_checkpoints::Event for TestInput {
        type Time = Time;

        fn time(&self) -> Self::Time {
            Time(1)
        }
    }
    impl Input for TestInput {
        fn event_id(&self) -> u128 {
            self.id
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }

    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: Time,
        value: usize,
    }

    impl Snapshot for TestSnapshot {
        type Time = Time;

        fn time(&self) -> &Self::Time {
            &self.time
        }
        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    impl ApplyEvents<TestInput> for TestSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestInput) -> Self {
            Self::default()
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Self::Time, TestInput>) {
            self.value += batch.events.map(|event| event.value).sum::<usize>();
        }
    }

    #[derive(Clone)]
    struct RecordingWrapper(Arc<AtomicUsize>);

    impl ApplyWrapper<TestSnapshot, TestInput> for RecordingWrapper {
        fn apply_event_batch(&mut self, batch: EventBatch<'_, '_, Time, TestInput>, apply_inner: &mut ApplyInner<'_, TestSnapshot>) {
            apply_inner.apply_event_batch(batch);
            self.0.store(apply_inner.snapshot().value, Ordering::Relaxed);
        }
    }

    fn config() -> ConTimeConfig<Time> {
        ConTimeConfig {
            router_count: 1,
            worker_count: 1,
            placement: contime_router::Placement::default(),
            pruning_interval: std::time::Duration::from_millis(100),

            history_retention: Time(0),
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
    fn receiver_closure_reports_admission_and_idle_wait_reports_application() {
        let observed = Arc::new(AtomicUsize::new(0));
        let contime =
            ConTime::<TestInput, TestSnapshot, RecordingWrapper>::start(config(), RecordingWrapper(Arc::clone(&observed))).unwrap();
        let (sender, receiver) = unbounded::<RejectionMessage<RejectionReason>>();

        contime.send([TestInput { id: 1, value: 5 }], sender.clone()).unwrap();
        contime.send([TestInput { id: 2, value: 7 }], sender.clone()).unwrap();
        drop(sender);

        assert_eq!(receiver.into_iter().collect::<Vec<_>>(), Vec::new());
        assert_eq!(observed.load(Ordering::Relaxed), 0);
        contime.advance_to(Time(1)).unwrap();
        contime.wait_until_idle(Duration::from_secs(2)).unwrap();
        assert_eq!(observed.load(Ordering::Relaxed), 12);
        contime.shutdown();
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_send() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/send/prepare_and_forward_1000", |bencher| {
            bencher.iter_batched(
                || {
                    let inputs = (0..1_000).map(|id| TestInput { id, value: 1 }).collect::<Vec<_>>();
                    let (rejection_sender, rejection_receiver) = unbounded();
                    let (output, output_receiver) = unbounded::<RouterMessage<TestInput, TestSnapshot>>();
                    (inputs, rejection_sender, rejection_receiver, output, output_receiver)
                },
                |(inputs, rejection_sender, rejection_receiver, output, output_receiver)| {
                    send_to(&output, inputs, rejection_sender).unwrap();
                    std::hint::black_box((rejection_receiver, output, output_receiver))
                },
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
