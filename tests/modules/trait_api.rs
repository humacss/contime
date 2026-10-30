use contime::{ContimeModule, EventId, SnapshotApply, SnapshotId};

struct Written {
    time: i64,
    tenant: String,
    list: u64,
    text: String,
}
impl contime::checkpoints::Event for Written {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl EventId for Written {
    fn event_id(&self) -> u128 {
        crate::test_id("Written", &(self.time, &self.tenant, self.list, &self.text))
    }
}
#[derive(Clone, Default)]

struct Document {
    time: i64,
    tenant: String,
    list: u64,
    text: String,
    counts: Vec<u64>,
}
impl contime::checkpoints::Snapshot for Document {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl Document {
    fn id_for(tenant: &String, list: &u64) -> u128 {
        crate::test_id("Document", &(tenant, list))
    }
}
impl contime::SnapshotId for Document {
    fn snapshot_id(&self) -> u128 {
        Self::id_for(&self.tenant, &self.list)
    }
}
impl SnapshotApply for Document {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<Written>(|event, emit| {
            emit(Document::id_for(&event.tenant, &event.list));
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if let Some(event) = event.get::<Written>() {
            let mut snapshot = Self::default();
            {
                snapshot.tenant = (event.tenant).clone();
                snapshot.list = event.list;
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        assert_eq!(batch.snapshot_id(), self.snapshot_id());
        self.counts.push(batch.history_event_count());
        let time = *batch.time();
        for event in batch.events::<Written>() {
            assert_eq!(event.time, time);
            self.text.push_str(&event.text);
        }
    }
}
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Written], snapshots = [Document])]
struct Documents;

#[test]
fn handwritten_traits_replay_borrowed_payloads_and_composite_keys() {
    let contime = contime::ConTime::<Documents>::start(super::config()).unwrap();
    let event = |time, text: &str| Written { time, tenant: "tenant".into(), list: 7, text: text.into() };
    contime.apply([event(10, "c").into(), event(-10, "a").into()]).unwrap();
    contime.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    contime.apply([event(0, "b").into()]).unwrap();
    contime.wait_until_idle(std::time::Duration::from_secs(3)).unwrap();
    let snapshots = contime.query_at(10, [Document::id_for(&("tenant".into()), &(7))]).unwrap();
    let DocumentsSnapshotLanes::Document(snapshot) = &*snapshots[0];
    assert_eq!(snapshot.text, "abc");
    assert_eq!(snapshot.counts, [0, 1, 2]);
    assert!(contime.errors().is_empty());
    contime.shutdown();
}

struct GlobalChange {
    time: i64,
    id: u128,
}
impl contime::checkpoints::Event for GlobalChange {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl EventId for GlobalChange {
    fn event_id(&self) -> u128 {
        self.id
    }
}
#[derive(Clone, Default)]

struct GlobalState {
    time: i64,
    count: usize,
}
impl contime::checkpoints::Snapshot for GlobalState {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl GlobalState {
    fn id_for() -> u128 {
        99
    }
}
impl contime::SnapshotId for GlobalState {
    fn snapshot_id(&self) -> u128 {
        Self::id_for()
    }
}
impl SnapshotApply for GlobalState {
    fn define(events: &mut contime::EventSetBuilder<Self>) {
        events.event::<GlobalChange>(|_event, emit| {
            emit(GlobalState::id_for());
        });
    }
    fn create(snapshot_id: u128, event: contime::EventRef<'_>) -> Self {
        if event.get::<GlobalChange>().is_some() {
            let snapshot = Self::default();
            {
                if contime::SnapshotId::snapshot_id(&snapshot) == snapshot_id {
                    return snapshot;
                }
            }
        }
        panic!("event does not initialize the requested snapshot")
    }
    fn apply_batch(&mut self, batch: impl contime::ApplyContext<Self>) {
        self.count += batch.events::<GlobalChange>().count();
    }
}
#[test]
fn empty_key_routes_every_event_to_the_same_snapshot() {
    let event = GlobalChange { time: -1, id: 5 };
    let set = contime::module::EventSet::<GlobalState>::new();
    let mut ids = Vec::new();
    set.snapshot_ids(contime::EventRef::new(&event), &mut |id| ids.push(id));
    assert_eq!(ids, [GlobalState::default().snapshot_id()]);
    let mut snapshot = set.create(ids[0], contime::EventRef::new(&event)).unwrap();
    let mut events = [&event].into_iter();
    contime::module::apply_batch(
        &mut snapshot,
        contime::checkpoints::ApplyBatch { snapshot_id: ids[0], time: -1, history_event_count: 0, events: &mut events },
    );
    assert_eq!(snapshot.count, 1);
}

#[test]
fn low_level_application_retains_non_static_payload_support() {
    use contime::checkpoints::{ApplyBatch, ApplyEvents, Event, Snapshot};
    struct Borrowed<'a> {
        text: &'a str,
    }
    impl Event for Borrowed<'_> {
        type Time = i64;
        fn time(&self) -> i64 {
            0
        }
    }
    #[derive(Clone, Default)]
    struct State {
        time: i64,
        text: String,
    }
    impl Snapshot for State {
        type Time = i64;
        fn time(&self) -> &i64 {
            &self.time
        }
        fn set_time(&mut self, time: i64) {
            self.time = time;
        }
    }
    impl<'a> ApplyEvents<Borrowed<'a>> for State {
        fn create(_: u128, _: &Borrowed<'a>) -> Self {
            Self::default()
        }
        fn apply_events(&mut self, batch: ApplyBatch<'_, '_, i64, Borrowed<'a>>) {
            for event in batch.events {
                self.text.push_str(event.text);
            }
        }
    }
    let text = String::from("borrowed locally");
    let event = Borrowed { text: &text };
    let mut state = State::create(3, &event);
    let mut events = [&event].into_iter();
    state.apply_events(ApplyBatch { snapshot_id: 3, time: 0, history_event_count: 0, events: &mut events });
    assert_eq!(state.text, text);
}
