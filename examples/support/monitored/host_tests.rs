use super::super::exit_tests::{self, Observed};
use super::*;
#[path = "../../cases/counter/mod.rs"]
mod counter;
use counter::setup::CounterExample;

fn options() -> Options {
    Options {
        snapshots: 1,
        workers: 1,
        rate: 1_000,
        maximum: false,
        headless: true,
        duration: Some(Duration::from_millis(30)),
        ..Options::default()
    }
}
fn events(_: Options, start: u64, count: u64, time: Time) -> impl Iterator<Item = counter::model::CounterChanged> {
    (start..start + count).map(move |id| CounterExample::event((id + 1).into(), time, vec![1], 1))
}
#[cfg(feature = "runtime")]
#[test]
fn standalone_runtime_exits_after_validation_without_teardown() {
    let name = concat!(module_path!(), "::standalone_runtime_exits_after_validation_without_teardown");
    let name = name.split_once("::").unwrap().1;
    exit_tests::assert_fast_exit(name, |marker| {
        let options = options();
        let backend = Observed::new(backend::runtime::<CounterExample>(&options).unwrap(), marker);
        run_backend::<CounterExample, _, _, _>(backend, options, events, Delivery::Forward, true).unwrap();
    });
}
#[cfg(feature = "manual")]
#[test]
fn standalone_manual_exits_after_validation_without_teardown() {
    let name = concat!(module_path!(), "::standalone_manual_exits_after_validation_without_teardown");
    let name = name.split_once("::").unwrap().1;
    exit_tests::assert_fast_exit(name, |marker| {
        let options = options();
        let backend = Observed::new(backend::Manual::<CounterExample>::new(&options).unwrap(), marker);
        run_backend::<CounterExample, _, _, _>(backend, options, events, Delivery::Forward, true).unwrap();
    });
}

#[cfg(feature = "manual")]
#[test]
fn reusable_manual_runner_returns_validated_summary_and_tears_down() {
    let marker = exit_tests::marker("manual-return");
    let _ = std::fs::remove_file(&marker);
    let options = options();
    let backend = Observed::new(backend::Manual::<CounterExample>::new(&options).unwrap(), marker.clone());
    let summary = run_backend::<CounterExample, _, _, _>(backend, options, events, Delivery::Forward, false).unwrap();
    assert_eq!(summary.events, summary.completed);
    assert!(marker.exists(), "reusable runner must release its backend");
    std::fs::remove_file(marker).unwrap();
}
#[cfg(feature = "manual")]
#[test]
fn standalone_validation_failure_returns_error_and_tears_down() {
    let marker = exit_tests::marker("manual-error");
    let _ = std::fs::remove_file(&marker);
    let mut options = options();
    let backend = Observed::new(backend::Manual::<CounterExample>::new(&options).unwrap(), marker.clone());
    options.rate = 0;
    assert!(run_backend::<CounterExample, _, _, _>(backend, options, events, Delivery::Forward, true).is_err());
    assert!(marker.exists(), "failed standalone run must release its backend");
    std::fs::remove_file(marker).unwrap();
}
