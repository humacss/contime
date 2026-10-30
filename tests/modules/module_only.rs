//! Public execution backends with module-only generated flat lanes.
#[path = "module_only/support.rs"]
mod support;
use contime::{ConTime, ContimeModule};
use std::{cell::RefCell, rc::Rc, time::Duration};
use support::*;

// This exhaustive match cannot compile if composition creates extra/nested variants.
fn lists(states: &[Box<AppSnapshotLanes>]) -> Vec<(u128, Vec<char>)> {
    states
        .iter()
        .filter_map(|s| match &**s {
            AppSnapshotLanes::OrderedList(s) => Some((s.id, s.values.clone())),
            AppSnapshotLanes::Counter(s) => {
                assert_eq!(s.count, 4);
                None
            }
        })
        .collect()
}
#[test]
fn flat_diamond_replays_late_events_and_deduplicates_types() {
    let contime: ConTime<App> = ConTime::start(config()).unwrap();
    contime.apply([added(10, 3, 'c').into(), added(-10, 1, 'a').into()]).unwrap();
    contime.wait_until_idle(Duration::from_secs(3)).unwrap();
    contime.apply([added(0, 2, 'b').into(), added(0, 4, 'x').into(), Cleared { at: 0, id: 99, list: 7 }.into()]).unwrap();
    contime.wait_until_idle(Duration::from_secs(3)).unwrap();
    let states = contime.query_at(10, [7, 9, COUNTER | 7, COUNTER | 9]).unwrap();
    let mut values = lists(&states);
    values.sort_by_key(|x| x.0);
    assert_eq!(values, [(7, vec!['b', 'x', 'c']), (9, vec!['a', 'b', 'x', 'c'])]);
    assert!(contime.errors().is_empty());
    assert!(contime.shutdown().workers.iter().all(|x| *x == contime::ThreadOutcome::Completed));
}
#[test]
fn manual_supports_negative_time_pruning_and_local_callbacks() {
    let mut contime = contime::manual::ManualConTime::<App>::new(manual_config());
    contime.apply([added(10, 3, 'c').into(), added(-10, 1, 'a').into()]).unwrap();
    contime.advance_to(10).unwrap();
    while contime.process().unwrap() {}
    contime.apply([added(0, 2, 'b').into()]).unwrap();
    while contime.process().unwrap() {}
    contime.prune_before(0).unwrap();
    while contime.process().unwrap() {}
    let result = Rc::new(RefCell::new(Vec::new()));
    let output = result.clone();
    contime
        .query_at(10, [7], move |states| {
            for s in states {
                if let AppSnapshotLanes::OrderedList(s) = *s {
                    output.borrow_mut().extend(s.values);
                }
            }
        })
        .unwrap();
    while contime.process().unwrap() {}
    assert_eq!(*result.borrow(), ['a', 'b', 'c']);
    assert!(contime.take_rejections().is_empty());
}
#[test]
fn explicit_names_deduplicate_import_aliases() {
    use contime::Input;
    let event: AliasesEventLanes = added(-1, 1, 'a').into();
    let mut ids = Vec::new();
    event.snapshot_ids(&mut |id| ids.push(id));
    assert_eq!(ids, [7, 9]);
    match event {
        AliasesEventLanes::Added(e) => assert_eq!(e.value, 'a'),
        AliasesEventLanes::Cleared(_) => panic!("wrong variant"),
    }
    let state = AliasesSnapshotLanes::from(OrderedList::default());
    let AliasesSnapshotLanes::OrderedList(_) = state;
}
#[test]
fn generated_lanes_have_the_same_size_as_flat_handwritten_enums() {
    #[allow(dead_code)]
    enum FlatEvents {
        Added(Added),
        Cleared(Cleared),
    }
    #[allow(dead_code)]
    enum FlatSnapshots {
        OrderedList(OrderedList),
        Counter(Counter),
    }
    assert_eq!(size_of::<AppEventLanes>(), size_of::<FlatEvents>());
    assert_eq!(size_of::<AppSnapshotLanes>(), size_of::<FlatSnapshots>());
    println!("flat event lane: {} bytes; flat snapshot lane: {} bytes", size_of::<AppEventLanes>(), size_of::<AppSnapshotLanes>());
}
#[derive(ContimeModule)]
#[contime_module(time=i64)]
struct Empty;
#[test]
fn empty_module_starts() {
    let c = ConTime::<Empty>::start(config()).unwrap();
    assert!(c.query_at(-1, [7]).unwrap().is_empty());
    c.shutdown();
}
#[derive(ContimeModule)]
#[contime_module(time=i64,snapshots=[OrderedList])]
struct MissingEvent;
#[test]
#[should_panic(expected = "absent from the module")]
fn missing_snapshot_event_is_rejected_before_runtime_starts() {
    let _ = ConTime::<MissingEvent>::start(config());
}

#[test]
fn minimum_timestamp_is_an_ordinary_event_time() {
    let mut c = contime::manual::ManualConTime::<App>::new(manual_config());
    c.apply([added(i64::MIN, 1, 'a').into(), added(i64::MIN, 1, 'a').into(), added(i64::MIN + 1, 2, 'b').into()]).unwrap();
    let result = Rc::new(RefCell::new(Vec::new()));
    let output = result.clone();
    c.query_at(i64::MIN + 1, [7], move |states| {
        let AppSnapshotLanes::OrderedList(s) = &*states[0] else { panic!("wrong lane") };
        output.borrow_mut().extend(s.values.iter().copied());
    })
    .unwrap();
    while c.process().unwrap() {}
    assert_eq!(*result.borrow(), ['a', 'b']);
    assert!(c.take_rejections().is_empty());
}
struct Number<T> {
    at: i64,
    value: T,
}
impl<T> contime::checkpoints::Event for Number<T> {
    type Time = i64;
    fn time(&self) -> i64 {
        self.at
    }
}
impl<T: Copy + Into<u128>> contime::EventId for Number<T> {
    fn event_id(&self) -> u128 {
        self.value.into()
    }
}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Small=Number<u32>,Large=Number<u64>,Small=Number<u32>])]
struct Numbers;
#[test]
fn distinct_generic_specializations_use_explicit_lane_names() {
    let _ = Numbers;
    let a: NumbersEventLanes = Number { at: -1, value: 3u32 }.into();
    let b: NumbersEventLanes = Number { at: 1, value: 4u64 }.into();
    match a {
        NumbersEventLanes::Small(e) => assert_eq!(e.value, 3),
        NumbersEventLanes::Large(_) => panic!("wrong lane"),
    }
    match b {
        NumbersEventLanes::Large(e) => assert_eq!(e.value, 4),
        NumbersEventLanes::Small(_) => panic!("wrong lane"),
    }
}

#[path = "module_only/local_data.rs"]
mod local_data;
