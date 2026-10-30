//! Repeated declarative expansions must keep nominal definitions distinct.
use contime::EventId;
use contime::{ContimeModule, SnapshotId};

macro_rules! domain {
    ($namespace:ident, $event:ident, $snapshot:ident) => {
        mod $namespace {
            use super::*;
            #[derive(Hash)]

            pub struct Added {
                pub time: i64,
                pub owner: u64,
                pub amount: i64,
            }
            impl contime::checkpoints::Event for Added {
                type Time = i64;
                fn time(&self) -> Self::Time {
                    self.time
                }
            }

            impl contime::EventId for Added {
                fn event_id(&self) -> u128 {
                    let event = self;
                    crate::test_id(std::any::type_name::<Self>(), event)
                }
            }

            #[derive(Clone, Default)]

            pub struct Total {
                pub time: i64,
                pub owner: u64,
                pub value: i64,
            }
            impl contime::checkpoints::Snapshot for Total {
                type Time = i64;
                fn time(&self) -> &Self::Time {
                    &self.time
                }
                fn set_time(&mut self, time: Self::Time) {
                    self.time = time;
                }
            }

            impl Total {
                pub fn id_for(owner: &u64) -> u128 {
                    crate::test_id(std::any::type_name::<Self>(), &(owner,))
                }
            }
            impl contime::SnapshotId for Total {
                fn snapshot_id(&self) -> u128 {
                    Self::id_for(&self.owner)
                }
            }
            impl contime::SnapshotApply for Total {
                fn define(events: &mut contime::EventSetBuilder<Self>) {
                    events.event::<Added>(|event, emit| {
                        emit(<Total>::id_for(&(event.owner)));
                    });
                }
                fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
                    if let Some(event) = event.get::<Added>() {
                        let mut snapshot = Self::default();
                        {
                            snapshot.owner = event.owner;
                            if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                                return snapshot;
                            }
                        }
                    }
                    panic!("event does not initialize the requested snapshot")
                }
                fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
                    {
                        for event in batch.events::<Added>() {
                            self.value += event.amount;
                        }
                    }
                }
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
    assert_ne!(left::Total::default().snapshot_id(), right::Total::default().snapshot_id());
    let mut contime = contime::manual::ManualConTime::<App>::new(contime::manual::ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(32).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    contime.apply([AppEventLanes::from(left), AppEventLanes::from(right)]).unwrap();
    let values = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let output = values.clone();
    contime
        .query_at(0, [left::Total::id_for(&(7)), right::Total::id_for(&(7))], move |states| {
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
