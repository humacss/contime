use crossbeam_channel::Sender;

use crate::{AdvanceOutput, ApiError};

traitstub::deps! {
    forward[O](output: O) -> Result<(), source::ApiError>;
}

/// Queues one advancement request with the caller's completion sender.
///
/// Success acknowledges enqueueing. Downstream adapters define advancement and
/// release all completion sender clones when their work reaches that boundary.
///
/// # Errors
///
/// Returns [`ApiError::OutputChannelClosed`] if the downstream receiver is closed.
pub fn send_advance_to<O, T>(output: &Sender<O>, time: T, completion: Sender<()>) -> Result<(), ApiError>
where
    O: AdvanceOutput<T>,
{
    send_advance_to_with_deps::<O, T>(&Deps(|message| output.send(message).map_err(|_| ApiError::OutputChannelClosed)), time, completion)
}

fn send_advance_to_with_deps<O, T>(deps: &impl calls::forward<O>, time: T, completion: Sender<()>) -> Result<(), ApiError>
where
    O: AdvanceOutput<T>,
{
    deps.forward(O::advance(time, completion))
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::Criterion;
    use crossbeam_channel::{unbounded, Sender, TryRecvError};

    use super::*;

    traitstub::stubs! {
        forward => |_| Ok(());
    }

    struct AdvanceMessage {
        time: u64,
        completion: Sender<()>,
    }

    impl AdvanceOutput<u64> for AdvanceMessage {
        fn advance(time: u64, completion: Sender<()>) -> Self {
            Self { time, completion }
        }
    }

    #[test]
    fn shared_defaults_complete_without_retaining_the_sender() {
        let (completion, done) = unbounded();
        assert_eq!(send_advance_to_with_deps::<AdvanceMessage, _>(&Defs, 50, completion), Ok(()));
        assert_eq!(done.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn send_advance_forwards_time_and_completion() {
        let (output, input) = unbounded();
        let (completion, done) = unbounded();

        super::send_advance_to::<AdvanceMessage, _>(&output, 50, completion).unwrap();

        let message = input.recv().unwrap();
        assert_eq!(message.time, 50);
        drop(message.completion);
        assert_eq!(done.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn send_advance_reports_a_closed_output_channel() {
        let (output, input) = unbounded::<AdvanceMessage>();
        let (completion, _done) = unbounded();
        drop(input);

        assert_eq!(super::send_advance_to(&output, 50, completion), Err(ApiError::OutputChannelClosed));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_send_advance() {
        let (output, input) = unbounded::<AdvanceMessage>();
        let mut criterion = Criterion::default();
        criterion.bench_function("api/send_advance_to", |bencher| {
            bencher.iter(|| {
                let (completion, _done) = unbounded();
                super::send_advance_to(&output, black_box(50), completion).unwrap();
                black_box(input.recv().unwrap());
            });
        });
        criterion.final_summary();
    }
}
