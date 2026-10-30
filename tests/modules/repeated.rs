//! Repeated declarative expansions must keep nominal definitions distinct.
use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};

// Keep nested derive attribute bodies intact across rustfmt runs.
#[rustfmt::skip]
macro_rules! domain {
    ($namespace:ident, $event:ident, $snapshot:ident) => {
        mod $namespace {
            use super::*;
            #[derive(Hash, ContimeEvent)]
#[contime_event(id = |event| crate::test_id(std::any::type_name::<Self>(), event))]
            pub struct Added {
                pub time: i64,
                pub owner: u64,
                pub amount: i64,
            }
            #[derive(ContimeSnapshot)]
            #[contime_snapshot(id = |owner| crate::test_id(std::any::type_name::<Self>(), &(owner,)), events = [Added], apply = {
                for event in batch.events {
                    let TotalEvent::Added(event) = event;
                    self.value += event.amount;
                }
            })]
            pub struct Total {
                pub time: i64,
                pub owner: u64,
                pub value: i64,
            }
            use Added as $event;
            use Total as $snapshot;
            #[derive(ContimeModule)]
            #[contime_module(time = i64, events = [$event], snapshots = [$snapshot])]
            pub struct Module;
        }
    };
}
domain!(left, LeftAdded, LeftTotal);
domain!(right, RightAdded, RightTotal);

#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [left::Module, right::Module])]
struct App;

#[test]
fn repeated_macro_definitions_compose_without_collisions_or_lost_lanes() {
    let _ = (left::Module, right::Module);
    let _ = (std::mem::size_of::<left::ModuleEventLanes>(), std::mem::size_of::<right::ModuleEventLanes>());
    let _ = (left::ModuleSnapshotLanes::from(left::Total::default()), right::ModuleSnapshotLanes::from(right::Total::default()));
    let left = left::Added { time: -1, owner: 7, amount: 2 };
    let right = right::Added { time: -1, owner: 7, amount: 3 };
    assert_ne!(left.event_id(), right.event_id());
    assert_ne!(left::Total::snapshot_id(7), right::Total::snapshot_id(7));
    let mut contime = contime::manual::ManualConTime::<App>::new(contime::manual::ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(32).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    contime.apply([AppEventLanes::from(left), AppEventLanes::from(right)]).unwrap();
    let values = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let output = values.clone();
    contime
        .query_at(0, [left::Total::snapshot_id(7), right::Total::snapshot_id(7)], move |states| {
            *output.borrow_mut() = states
                .into_iter()
                .map(|state| match *state {
                    AppSnapshotLanes::LeftTotal(state) => state.value,
                    AppSnapshotLanes::RightTotal(state) => state.value,
                })
                .collect();
        })
        .unwrap();
    while contime.process().unwrap() {}
    values.borrow_mut().sort_unstable();
    assert_eq!(*values.borrow(), [2, 3]);
    assert!(contime.take_rejections().is_empty());
}
