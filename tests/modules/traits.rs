use contime::ContimeModule;
use std::{cell::RefCell, rc::Rc};

mod model {
    use contime::{ContimeEvent, ContimeModule, ContimeSnapshot};
    use std::hash::{Hash, Hasher};

    // Intentionally implements neither Clone nor Hash.
    pub struct Payload(pub String);

    #[derive(ContimeEvent)]
    #[contime_event(id = self.id)]
    pub struct MessageAdded {
        pub id: u128,
        pub time: i64,
        pub owner: u64,
        pub payload: Payload,
    }

    #[derive(ContimeEvent)]
    #[contime_event(hash = false)]
    pub struct Hashed {
        pub time: i64,
        pub owner: u64,
        pub amount: usize,
    }
    impl Hash for Hashed {
        fn hash<H: Hasher>(&self, state: &mut H) {
            (self.time, self.owner, self.amount).hash(state);
        }
    }

    #[derive(ContimeSnapshot)]
    #[contime_snapshot(clone = false, default = false, id = [self.owner], events = [MessageAdded, Hashed], apply = {
        for event in batch.events {
            self.value += match event {
                TotalEvent::MessageAdded(event) => event.payload.0.len(),
                TotalEvent::Hashed(event) => event.amount,
            };
        }
    })]
    pub struct Total {
        pub time: i64,
        pub owner: u64,
        pub value: usize,
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
        .query_at(0, [model::Total::snapshot_id(7)], move |states| {
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
