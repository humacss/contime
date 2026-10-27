use crossbeam_channel::{unbounded, Sender};

use crate::{AdvanceOutput, ApiError};

traitstub::deps! {
    send[T](time: T, completion: crossbeam_channel::Sender<()>) -> Result<(), source::ApiError>;
}

/// Forwards `time` and blocks until all downstream completion senders close.
///
/// The downstream adapter defines what advancement completion means. Received
/// unit messages are discarded; only sender closure finishes the wait.
///
/// # Errors
///
/// Returns [`ApiError::OutputChannelClosed`] if the request cannot be queued.
pub fn advance_to<O, T>(output: &Sender<O>, time: T) -> Result<(), ApiError>
where
    O: AdvanceOutput<T>,
{
    advance_to_with_deps(&Deps(|time, completion| crate::send_advance_to::send_advance_to(output, time, completion)), time)
}

fn advance_to_with_deps<T>(deps: &impl calls::send<T>, time: T) -> Result<(), ApiError> {
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
    use crossbeam_channel::{bounded, RecvTimeoutError};

    use super::*;

    traitstub::stubs! {
        send => |_, _| Ok(());
    }

    #[test]
    fn synchronous_advance_finishes_when_the_completion_sender_closes() {
        assert_eq!(advance_to_with_deps(&Defs, 50_u64), Ok(()));
    }

    #[test]
    fn synchronous_advance_waits_for_downstream_sender_clones() {
        let (retained, clones) = bounded(1);
        let (finished, result) = bounded(1);
        thread::scope(|scope| {
            let worker = scope.spawn(move || {
                let deps = traitstub::defs! {
                    send => |time: u64, completion: Sender<()>| {
                        assert_eq!(time, 50);
                        completion.send(()).unwrap();
                        retained.send(completion.clone()).unwrap();
                        Ok(())
                    };
                };
                finished.send(advance_to_with_deps(&deps, 50)).unwrap();
            });
            let completion = clones.recv_timeout(Duration::from_secs(5)).unwrap();
            // A unit message is not completion while a downstream clone exists.
            assert_eq!(result.recv_timeout(Duration::from_millis(10)), Err(RecvTimeoutError::Timeout));
            drop(completion);
            assert_eq!(result.recv_timeout(Duration::from_secs(5)).unwrap(), Ok(()));
            worker.join().unwrap();
        });
    }

    #[test]
    fn propagates_enqueue_errors() {
        let deps = traitstub::defs! {
            send => |_: u64, _: Sender<()>| Err(ApiError::OutputChannelClosed);
        };
        assert_eq!(advance_to_with_deps(&deps, 50), Err(ApiError::OutputChannelClosed));
    }

    struct AdvanceMessage {
        time: String,
        completion: Sender<()>,
    }

    impl AdvanceOutput<String> for AdvanceMessage {
        fn advance(time: String, completion: Sender<()>) -> Self {
            Self { time, completion }
        }
    }

    #[test]
    fn public_advance_binds_the_real_sender() {
        let (output, requests) = unbounded::<AdvanceMessage>();
        thread::scope(|scope| {
            let downstream = scope.spawn(move || {
                let request = requests.recv().unwrap();
                assert_eq!(request.time, "tick");
                drop(request.completion);
            });
            assert_eq!(advance_to(&output, "tick".into()), Ok(()));
            downstream.join().unwrap();
        });
    }

    #[test]
    fn public_advance_reports_a_closed_output_channel() {
        let (output, requests) = unbounded::<AdvanceMessage>();
        drop(requests);
        assert_eq!(advance_to(&output, "tick".into()), Err(ApiError::OutputChannelClosed));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_synchronous_advance() {
        let mut criterion = Criterion::default();
        criterion.bench_function("api/advance_to/immediate_completion", |bencher| {
            bencher.iter(|| advance_to_with_deps(&Defs, black_box(50_u64)).unwrap());
        });
        criterion.final_summary();
    }
}
