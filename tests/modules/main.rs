mod external;
mod generic;
mod identity;
mod repeated;
mod routing;
mod traits;

use std::{cell::RefCell, rc::Rc, time::Duration};

use contime::{ConTime, ConTimeConfig, ContimeEvent, ContimeModule, ContimeSnapshot};

#[derive(Hash, ContimeEvent)]
#[contime_event(id = |event| crate::test_id(std::any::type_name::<Self>(), event))]
struct Added {
    time: i64,
    counter_id: u128,
    amount: i64,
}

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |counter_id| crate::test_id(std::any::type_name::<Self>(), &(counter_id,)), events = [Added], apply = {
    for event in batch.events {
        let CounterEvent::Added(event) = event;
        self.value += event.amount;
    }
})]
struct Counter {
    time: i64,
    counter_id: u128,
    value: i64,
}

#[derive(ContimeSnapshot)]
#[contime_snapshot(id = |counter_id| crate::test_id(std::any::type_name::<Self>(), &(counter_id,)), events = [Added], apply = {
    for event in batch.events {
        let CounterStatisticsEvent::Added(_) = event;
        self.count += 1;
    }
})]
struct CounterStatistics {
    time: i64,
    counter_id: u128,
    count: usize,
}

#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added], snapshots = [Counter])]
#[allow(dead_code)]
struct Counters;

#[derive(ContimeModule)]
#[contime_module(time = i64, snapshots = [CounterStatistics], modules = [Counters])]
#[allow(dead_code)]
struct Statistics;

// Counters is reachable twice. Each event and snapshot lane must occur once.
#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [Counters, Statistics])]
struct App;

fn config() -> ConTimeConfig<i64> {
    ConTimeConfig {
        router_count: 2,
        worker_count: 3,
        placement: contime::Placement::default(),
        pruning_interval: Duration::from_secs(60),
        history_retention: i64::MAX,
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    }
}

fn added(time: i64, amount: i64) -> AppEventLanes {
    Added { time, counter_id: 7, amount }.into()
}

fn values(states: &[Box<AppSnapshotLanes>]) -> (i64, usize) {
    let mut value = None;
    let mut count = None;
    for snapshot in states {
        match &**snapshot {
            AppSnapshotLanes::Counter(snapshot) => {
                assert_eq!(snapshot.counter_id, 7);
                value = Some(snapshot.value);
            }
            AppSnapshotLanes::CounterStatistics(snapshot) => {
                assert_eq!(snapshot.counter_id, 7);
                count = Some(snapshot.count);
            }
        }
    }
    (value.unwrap(), count.unwrap())
}

fn ids() -> [u128; 2] {
    [Counter::snapshot_id(7), CounterStatistics::snapshot_id(7)]
}

#[test]
fn derives_supply_stable_ids_and_namespaced_snapshot_ids() {
    let first = Added { time: -10, counter_id: 7, amount: 3 };
    let duplicate = Added { time: -10, counter_id: 7, amount: 3 };
    let different = Added { time: -10, counter_id: 7, amount: 4 };
    assert_eq!(first.event_id(), duplicate.event_id());
    assert_ne!(first.event_id(), different.event_id());
    assert_ne!(ids()[0], ids()[1]);
    let cloned = Counter::default().clone();
    assert_eq!(cloned.value, 0);
}

#[test]
fn runtime_reconciles_negative_first_and_late_events_across_zero() {
    let history = ConTime::<App>::start(config()).unwrap();
    history.apply([added(-10, 3), added(0, 5), added(10, 7)]).unwrap();
    history.advance_to(10).unwrap();
    history.wait_until_idle(Duration::from_secs(3)).unwrap();
    assert_eq!(values(&history.query_at(-10, ids()).unwrap()), (3, 1));
    assert_eq!(values(&history.query_at(0, ids()).unwrap()), (8, 2));
    // A correction predating the first event must remain admissible.
    history.apply([added(-20, 11), added(-10, 3)]).unwrap();
    history.wait_until_idle(Duration::from_secs(3)).unwrap();
    assert_eq!(values(&history.query_at(-20, ids()).unwrap()), (11, 1));
    assert_eq!(values(&history.query_at(0, ids()).unwrap()), (19, 3));
    assert_eq!(values(&history.query_at(10, ids()).unwrap()), (26, 4));
    assert!(history.errors().is_empty());
    let report = history.shutdown();
    assert!(report.workers.iter().all(|outcome| *outcome == contime::ThreadOutcome::Completed));
}

#[test]
fn runtime_accepts_minimum_timestamp_and_deduplicates_identical_inputs() {
    let history = ConTime::<App>::start(config()).unwrap();
    history.apply([added(i64::MIN, 2), added(i64::MIN, 2), added(i64::MIN + 1, 3)]).unwrap();
    history.wait_until_idle(Duration::from_secs(3)).unwrap();
    assert_eq!(values(&history.query_at(i64::MIN, ids()).unwrap()), (2, 1));
    assert_eq!(values(&history.query_at(i64::MIN + 1, ids()).unwrap()), (5, 2));
    assert!(history.errors().is_empty());
    history.shutdown();
}

#[test]
fn manual_uses_the_same_module_and_primitive_time_with_host_driven_queries() {
    let mut history = contime::manual::ManualConTime::<App>::new(contime::manual::ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(32).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    history.apply([added(-5, 2), added(0, 3), added(-5, 2)]).unwrap();
    history.advance_to(0).unwrap();
    let observed = Rc::new(RefCell::new(None));
    let output = Rc::clone(&observed);
    history.query_at(0, ids(), move |states| *output.borrow_mut() = Some(values(&states))).unwrap();
    assert!(observed.borrow().is_none());
    for _ in 0..1000 {
        if !history.process().unwrap() {
            break;
        }
    }
    assert_eq!(*observed.borrow(), Some((5, 2)));
    assert!(history.take_rejections().is_empty());
}

#[test]
fn module_time_mismatches_and_cycles_are_compile_errors() {
    trybuild::TestCases::new().compile_fail("tests/modules/ui/*.rs");
}

#[cfg(feature = "monitor")]
#[test]
fn a_module_instance_can_be_monitored_without_unwrapping_its_runtime() {
    let history = ConTime::<App>::start(config()).unwrap();
    let mut monitor = contime::monitor::WorkerMonitor::attach(&history);
    monitor.begin(Duration::ZERO).unwrap();
    monitor.cancel();
    history.shutdown();
}

fn test_id<T: std::hash::Hash + ?Sized>(namespace: &str, value: &T) -> u128 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    namespace.hash(&mut hash);
    value.hash(&mut hash);
    u128::from(hash.finish())
}
