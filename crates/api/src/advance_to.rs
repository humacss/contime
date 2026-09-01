use crossbeam_channel::{unbounded, Sender};

use crate::{AdvanceOutput, ApiError};

trait Deps<T> {
    type Error;

    fn send(&self, time: T, completion: Sender<()>) -> Result<(), Self::Error>;
}

struct DefaultDeps<'a, O> {
    output: &'a Sender<O>,
}

impl<O, T> Deps<T> for DefaultDeps<'_, O>
where
    O: AdvanceOutput<T>,
{
    type Error = ApiError;

    fn send(&self, time: T, completion: Sender<()>) -> Result<(), ApiError> {
        crate::send_advance_to::send_advance_to(self.output, time, completion)
    }
}

pub fn advance_to<O, T>(output: &Sender<O>, time: T) -> Result<(), ApiError>
where
    O: AdvanceOutput<T>,
{
    advance_with_deps(&DefaultDeps { output }, time)
}

fn advance_with_deps<D, T>(deps: &D, time: T) -> Result<(), D::Error>
where
    D: Deps<T>,
{
    let (completion, receiver) = unbounded();
    deps.send(time, completion)?;
    receiver.into_iter().for_each(drop);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::thread;
    use std::time::Duration;

    use criterion::Criterion;
    use crossbeam_channel::Sender;

    use super::{advance_with_deps, Deps};

    #[derive(Debug, Eq, PartialEq)]
    struct StubError;

    struct Immediate;

    impl Deps<u64> for Immediate {
        type Error = StubError;

        fn send(&self, _time: u64, _completion: Sender<()>) -> Result<(), StubError> {
            Ok(())
        }
    }

    struct Delayed;

    impl Deps<u64> for Delayed {
        type Error = StubError;

        fn send(&self, _time: u64, completion: Sender<()>) -> Result<(), StubError> {
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(10));
                drop(completion);
            });
            Ok(())
        }
    }

    #[test]
    fn synchronous_advance_finishes_when_the_completion_sender_closes() {
        assert_eq!(advance_with_deps(&Immediate, 50), Ok(()));
    }

    #[test]
    fn synchronous_advance_waits_for_downstream_sender_clones() {
        let started = std::time::Instant::now();

        assert_eq!(advance_with_deps(&Delayed, 50), Ok(()));

        assert!(started.elapsed() >= Duration::from_millis(10));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_synchronous_advance() {
        let mut criterion = Criterion::default();
        criterion.bench_function("api/advance_to/immediate_completion", |bencher| {
            bencher.iter(|| black_box(advance_with_deps(&Immediate, black_box(50)).unwrap()));
        });
        criterion.final_summary();
    }
}
