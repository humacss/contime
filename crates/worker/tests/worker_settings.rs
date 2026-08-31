use std::sync::{Arc, Mutex};
use std::time::Duration;

use contime_worker::{
    work, ApplyBatch, CheckpointResult, Checkpoints, CheckpointsCreated, EventInsert, Events, EventsCreated, RoutedInput, WorkerConfig,
    WorkerInput, WorkerRejection,
};
use crossbeam_channel::unbounded;

struct TestInput(u128);

impl WorkerInput for TestInput {
    fn input_id(&self) -> u128 {
        self.0
    }

    fn conservative_size(&self) -> u64 {
        32
    }
}

#[derive(Default)]
struct TestEvents(Vec<u128>);

impl Events<TestInput> for TestEvents {
    type Config = ();
    type Rejection = ();

    fn create(_snapshot_id: u128, _config: &(), _limit: u64) -> Option<EventsCreated<Self>> {
        Some(EventsCreated { events: Self::default(), retained_bytes_delta: 0 })
    }

    fn insert(&mut self, input: TestInput, _limit: u64) -> EventInsert<()> {
        self.0.push(input.0);
        EventInsert { retained_bytes_delta: 32, changed: true, rejections: Vec::new() }
    }
}

struct TestCheckpoints;

impl Checkpoints<TestEvents> for TestCheckpoints {
    type Config = ();
    type Context = Arc<Mutex<Vec<Vec<u128>>>>;

    fn create(_snapshot_id: u128, _config: &(), _limit: u64) -> CheckpointsCreated<Self> {
        CheckpointsCreated { checkpoints: Self, retained_bytes_delta: 0 }
    }

    fn update(&mut self, events: &TestEvents, context: &mut Self::Context, _limit: u64) -> CheckpointResult {
        context.lock().unwrap().push(events.0.clone());
        CheckpointResult { retained_bytes_delta: 0 }
    }
}

type TestCompletion = crossbeam_channel::Sender<Vec<WorkerRejection<()>>>;

fn batch(input_id: u128) -> ApplyBatch<TestInput, TestCompletion> {
    batch_for_snapshots(input_id, &[7])
}

fn batch_for_snapshots(input_id: u128, snapshot_ids: &[u128]) -> ApplyBatch<TestInput, TestCompletion> {
    let (completion, _responses) = unbounded();
    ApplyBatch {
        inputs: snapshot_ids.iter().map(|snapshot_id| RoutedInput { snapshot_id: *snapshot_id, input: TestInput(input_id) }).collect(),
        completion,
    }
}

fn config(replays_per_receive: usize) -> WorkerConfig {
    WorkerConfig {
        memory_limit: 1_000_000,
        maximum_dirty_age: Duration::from_secs(60),
        replays_per_receive,
        deadline_compaction_minimum: 4,
        deadline_compaction_multiplier: 2,
    }
}

fn run_hot_snapshot(replays_per_receive: usize) -> Vec<Vec<u128>> {
    let (sender, receiver) = unbounded();
    for input_id in 0..10 {
        sender.send(batch(input_id)).unwrap();
    }
    drop(sender);

    let context = Arc::new(Mutex::new(Vec::new()));
    work::<TestInput, TestEvents, TestCheckpoints, _>(receiver, config(replays_per_receive), (), (), Arc::clone(&context));

    Arc::try_unwrap(context).unwrap().into_inner().unwrap()
}

#[test]
fn zero_replays_per_receive_batches_hot_snapshot_until_disconnect() {
    assert_eq!(run_hot_snapshot(0), vec![(0..10).collect::<Vec<_>>()]);
}

#[test]
fn one_replay_per_receive_updates_hot_snapshot_after_every_batch() {
    let expected = (1..=10).map(|length| (0..length).collect::<Vec<_>>()).collect::<Vec<_>>();

    assert_eq!(run_hot_snapshot(1), expected);
}

#[test]
fn replay_budget_controls_updates_across_four_hot_snapshots() {
    fn replay_count(replays_per_receive: usize) -> usize {
        let (sender, receiver) = unbounded();
        for input_id in 0..10 {
            sender.send(batch_for_snapshots(input_id, &[0, 1, 2, 3])).unwrap();
        }
        drop(sender);

        let context = Arc::new(Mutex::new(Vec::new()));
        work::<TestInput, TestEvents, TestCheckpoints, _>(receiver, config(replays_per_receive), (), (), Arc::clone(&context));
        let replay_count = context.lock().unwrap().len();
        replay_count
    }

    assert_eq!(replay_count(0), 4);
    assert_eq!(replay_count(1), 13);
    assert_eq!(replay_count(4), 40);
}

#[test]
fn zero_dirty_age_replays_without_an_additional_input() {
    let (sender, receiver) = unbounded();
    let (completion, completed) = unbounded();
    let context = Arc::new(Mutex::new(Vec::new()));
    let worker_context = Arc::clone(&context);
    let mut worker_config = config(0);
    worker_config.maximum_dirty_age = Duration::ZERO;

    let worker = std::thread::spawn(move || {
        work::<TestInput, TestEvents, TestCheckpoints, _>(receiver, worker_config, (), (), worker_context);
    });

    sender.send(ApplyBatch { inputs: vec![RoutedInput { snapshot_id: 7, input: TestInput(1) }], completion }).unwrap();

    assert_eq!(completed.recv_timeout(Duration::from_secs(1)), Err(crossbeam_channel::RecvTimeoutError::Disconnected),);
    drop(sender);
    worker.join().unwrap();
    assert_eq!(*context.lock().unwrap(), vec![vec![1]]);
}
