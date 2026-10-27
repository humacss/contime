use contime_simulator::{MaximumConfig, Simulator, Workload};
use std::{num::NonZeroU64, time::Duration};
#[derive(Default)]
struct Words {
    expected: String,
}
impl Workload for Words {
    type Event = char;
    type Snapshot = String;
    type Expectation = String;
    type Error = &'static str;
    fn next_applications(&self) -> NonZeroU64 {
        NonZeroU64::new(1).unwrap()
    }
    fn generate(&mut self, _sequence: u64, _at: Duration) -> char {
        self.expected.push('a');
        'a'
    }
    fn expectation(&self) -> String {
        self.expected.clone()
    }
    fn validate(&self, expected: &String, actual: &[String]) -> Result<(), Self::Error> {
        if actual == std::slice::from_ref(expected) {
            Ok(())
        } else {
            Err("wrong words")
        }
    }
}
#[test]
fn custom_snapshot_without_counters_validates_captured_prefix() {
    let mut simulator = Simulator::new(Words::default(), MaximumConfig::default()).unwrap();
    assert!(simulator
        .produce(Duration::ZERO, |event| {
            assert_eq!(event, 'a');
            Ok::<_, ()>(())
        })
        .unwrap());
    let query = simulator.start_query(Duration::ZERO).unwrap();
    assert!(simulator.produce(Duration::ZERO, |_| Ok::<_, ()>(())).unwrap());
    simulator.complete_query(query, &["a".to_owned()], Duration::from_millis(1)).unwrap();
    assert_eq!(simulator.monitor().confirmed().events, 1);
    assert_eq!(simulator.monitor().submitted().events, 2);
}
#[test]
fn failed_validation_never_confirms_work() {
    let mut simulator = Simulator::new(Words::default(), MaximumConfig::default()).unwrap();
    simulator.produce(Duration::ZERO, |_| Ok::<_, ()>(())).unwrap();
    let query = simulator.start_query(Duration::ZERO).unwrap();
    assert!(simulator.complete_query(query, &["wrong".to_owned()], Duration::from_millis(1)).is_err());
    assert_eq!(simulator.monitor().confirmed().events, 0);
}
#[test]
fn failed_submission_poisoning_prevents_reporting_or_reusing_a_mutated_reference() {
    let mut simulator = Simulator::new(Words::default(), MaximumConfig::default()).unwrap();
    assert!(simulator.produce(Duration::ZERO, |_| Err("closed")).is_err());
    assert_eq!(simulator.monitor().submitted().events, 0);
    assert!(simulator.produce(Duration::ZERO, |_| Ok::<_, ()>(())).is_err());
    assert!(simulator.start_query(Duration::ZERO).is_err());
}

#[test]
fn all_operations_share_a_monotonic_clock() {
    let mut simulator = Simulator::new(Words::default(), MaximumConfig::default()).unwrap();
    simulator.start_query(Duration::from_secs(1)).unwrap();
    assert!(simulator.produce(Duration::ZERO, |_| Ok::<_, ()>(())).is_err());
    assert_eq!(simulator.workload().expected, "");
    let mut simulator = Simulator::new(Words::default(), MaximumConfig::default()).unwrap();
    simulator.produce(Duration::from_secs(1), |_| Ok::<_, ()>(())).unwrap();
    assert!(simulator.start_query(Duration::ZERO).is_err());
    assert!(simulator.sample(Duration::ZERO).is_err());
    let mut simulator = Simulator::new(Words::default(), MaximumConfig::default()).unwrap();
    simulator.sample(Duration::from_secs(1)).unwrap();
    assert!(simulator.produce(Duration::ZERO, |_| Ok::<_, ()>(())).is_err());
}
