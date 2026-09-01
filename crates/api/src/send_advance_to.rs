use crossbeam_channel::Sender;

use crate::{AdvanceOutput, ApiError};

trait Deps<O> {
    type Error;

    fn forward(&self, output: O) -> Result<(), Self::Error>;
}

struct DefaultDeps<'a, O> {
    output: &'a Sender<O>,
}

impl<O> Deps<O> for DefaultDeps<'_, O> {
    type Error = ApiError;

    fn forward(&self, output: O) -> Result<(), Self::Error> {
        self.output.send(output).map_err(|_| ApiError::OutputChannelClosed)
    }
}

pub fn send_advance_to<O, T>(output: &Sender<O>, time: T, completion: Sender<()>) -> Result<(), ApiError>
where
    O: AdvanceOutput<T>,
{
    send_with_deps(&DefaultDeps { output }, time, completion)
}

fn send_with_deps<D, O, T>(deps: &D, time: T, completion: Sender<()>) -> Result<(), D::Error>
where
    D: Deps<O>,
    O: AdvanceOutput<T>,
{
    deps.forward(O::advance(time, completion))
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::Criterion;
    use crossbeam_channel::{unbounded, Sender, TryRecvError};

    use crate::{AdvanceOutput, ApiError};

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
