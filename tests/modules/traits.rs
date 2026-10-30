use contime::ContimeModule;
use std::{cell::RefCell, rc::Rc};

mod model {
    use contime::ContimeModule;
    use std::hash::{Hash, Hasher};

    // Intentionally implements neither Clone nor Hash.
    pub struct Payload(pub String);

    pub struct MessageAdded {
        pub id: u128,
        pub time: i64,
        pub owner: u64,
        pub payload: Payload,
    }
    impl contime::checkpoints::Event for MessageAdded {
        type Time = i64;
        fn time(&self) -> Self::Time {
            self.time
        }
    }

    impl contime::EventId for MessageAdded {
        fn event_id(&self) -> u128 {
            self.id
        }
    }

    pub struct Hashed {
        pub time: i64,
        pub owner: u64,
        pub amount: usize,
    }
    impl contime::checkpoints::Event for Hashed {
        type Time = i64;
        fn time(&self) -> Self::Time {
            self.time
        }
    }

    impl contime::EventId for Hashed {
        fn event_id(&self) -> u128 {
            crate::test_id(std::any::type_name::<Self>(), self)
        }
    }

    impl Hash for Hashed {
        fn hash<H: Hasher>(&self, state: &mut H) {
            (self.time, self.owner, self.amount).hash(state);
        }
    }

    pub struct Total {
        pub time: i64,
        pub owner: u64,
        pub value: usize,
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
            events.event::<MessageAdded>(|event, emit| {
                emit(Total::id_for(&event.owner));
            });
            events.event::<Hashed>(|event, emit| {
                emit(Total::id_for(&event.owner));
            });
        }
        fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
            if let Some(event) = event.get::<MessageAdded>() {
                let mut snapshot = Self::default();
                {
                    snapshot.owner = event.owner;
                    if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                        return snapshot;
                    }
                }
            }
            if let Some(event) = event.get::<Hashed>() {
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
            for event in batch.events::<MessageAdded>() {
                self.value += event.payload.0.len();
            }
            for event in batch.events::<Hashed>() {
                self.value += event.amount;
            }
        }
    }

    impl Clone for Total {
        fn clone(&self) -> Self {
            Self { time: self.time, owner: self.owner, value: self.value }
        }
    }
    impl Default for Total {
        fn default() -> Self {
            Self { time: 0, owner: 0, value: 100 }
        }
    }

    use MessageAdded as Incoming;
    use Total as State;

    #[derive(ContimeModule)]
    #[contime_module(time = i64, events = [Incoming, Hashed], snapshots = [State])]
    #[allow(dead_code)]
    pub struct Messages;
}

#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [model::Messages])]
struct Application;

#[test]
fn qualified_modules_aliases_and_manual_trait_opt_outs_work_with_nonclone_events() {
    let mut history = contime::manual::ManualConTime::<Application>::new(contime::manual::ManualConfig {
        queue_capacity: std::num::NonZeroUsize::new(16).unwrap(),
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 2 },
    });
    history
        .apply([
            ApplicationEventLanes::from(model::MessageAdded { id: 1, time: -1, owner: 7, payload: model::Payload("abc".into()) }),
            model::Hashed { time: 0, owner: 7, amount: 2 }.into(),
        ])
        .unwrap();
    let observed = Rc::new(RefCell::new(None));
    let output = Rc::clone(&observed);
    history
        .query_at(0, [model::Total::id_for(&(7))], move |states| {
            let ApplicationSnapshotLanes::State(state) = &*states[0];
            *output.borrow_mut() = Some((state.owner, state.value));
        })
        .unwrap();
    for _ in 0..1000 {
        if !history.process().unwrap() {
            break;
        }
    }
    assert_eq!(*observed.borrow(), Some((7, 105)));
}
