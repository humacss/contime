//! Component integration: manually deliver messages and execute real worker steps.
//! Included by Core's test module to access its private adapters and admission.
//! No OS threads, clocks, retries, or alternative scheduling implementation.
use crate::checkpoints::*;
use crate::coordinator::admit;
use crate::frontier::Frontier;
use crate::input::prepare_inputs;
use crate::types::{CheckpointStorage, CheckpointStorageConfig};
use crate::{Advance, CompletionHandle, Input, Route, RouterBatch, RouterMessage, WorkerBatch, WorkerMessage};
use contime_worker::{Coordination, MessageWorker};
use crossbeam_channel::{never, unbounded, Sender};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Time(u64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime_worker::AdvanceTime for Time {
    fn saturating_sub(&self, retention: &Self) -> Self {
        Self(self.0.saturating_sub(retention.0))
    }
}
struct TestEvent {
    id: u128,
    time: Time,
}
impl contime_checkpoints::Event for TestEvent {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl Input for TestEvent {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(1);
    }
}
#[derive(Clone, Default)]
struct TestSnapshot {
    time: Time,
    sum: u64,
}
impl Snapshot for TestSnapshot {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl ApplyEvents<TestEvent> for TestSnapshot {
    fn create(_: u128, _: &TestEvent) -> Self {
        Self::default()
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, TestEvent>) {
        self.sum += batch.events.count() as u64;
    }
}
struct Publish {
    outputs: Sender<(Time, u64)>,
}
impl ApplyWrapper<TestSnapshot, TestEvent> for Publish {
    fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, Time, TestEvent>, inner: &mut ApplyInner<'_, TestSnapshot>) {
        let time = batch.time;
        inner.apply_event_batch(batch);
        self.outputs.send((time, inner.snapshot().sum)).unwrap();
    }
}
type TestWorker<'a> = MessageWorker<'a, WorkerMessage<TestEvent, TestSnapshot>, CheckpointStorage<TestEvent, TestSnapshot, Publish>>;
fn advance(time: Time) -> Advance<Time> {
    Advance { time, completion: unbounded().0 }
}
fn apply(events: Vec<TestEvent>) -> WorkerMessage<TestEvent, TestSnapshot> {
    route(RouterBatch { inputs: prepare_inputs(events), completion: CompletionHandle::new(unbounded().0) })
}
fn route(batch: RouterBatch<TestEvent>) -> WorkerMessage<TestEvent, TestSnapshot> {
    WorkerMessage::Apply(WorkerBatch {
        routes: batch.inputs.into_iter().map(|input| Route { snapshot_id: 1, input }).collect(),
        completion: batch.completion,
    })
}

#[rstest::rstest]
#[case::sparse_checkpoint(1, 100, 90, 80, 100)]
#[case::unbounded_checkpoint(1, 0, 90, 80, 100)]
#[case::same_timestamp(1, 100, 100, 80, 100)]
#[case::ten_workers(10, 100, 90, 80, 100)]
#[case::dense_checkpoint_control(1, 1, 90, 80, 100)]
#[case::earlier_cutoff_control(1, 100, 90, 5, 100)]
#[case::completed_cursor_before_cutoff(1, 100, 90, 80, 10)]
fn frontier_must_not_overtake_possible_replay_outputs(
    #[case] worker_count: usize,
    #[case] interval: u64,
    #[case] insertion_time: u64,
    #[case] cutoff: u64,
    #[case] initial_progress: u64,
) {
    let initial_target = Time(100);
    let expected_initial_outputs: Vec<_> =
        [(Time(10), 1), (Time(100), 2)].into_iter().filter(|(time, _)| time.0 <= initial_progress).collect();
    let expected_sum = 3;

    let messages = never();
    let registrations = never();
    let (reported, reports) = unbounded();
    let (published, publications) = unbounded();
    let mut workers: Vec<TestWorker<'_>> = (0..worker_count)
        .map(|id| {
            let reported = reported.clone();
            TestWorker::new(
                &messages,
                &registrations,
                CheckpointStorageConfig { checkpoints: CheckpointConfig { interval } },
                Publish { outputs: published.clone() },
                Some(Coordination {
                    router_count: 1,
                    report: Box::new(move |round, minimum| {
                        reported.send((round, id, minimum)).unwrap();
                    }),
                    pruned: Box::new(|_| {}),
                }),
            )
        })
        .collect();
    let mut frontier = Frontier::new(worker_count);
    workers[0].handle(apply(vec![TestEvent { id: 1, time: Time(10) }, TestEvent { id: 2, time: initial_target }]));
    workers[0].handle(WorkerMessage::Advance(advance(Time(initial_progress))));
    assert!(workers[0].step());
    assert!(!workers[0].step());
    assert_eq!(publications.try_iter().collect::<Vec<_>>(), expected_initial_outputs);
    workers[0].handle(WorkerMessage::Advance(advance(initial_target)));

    // Accept the late event before closing admission, but leave replay pending.
    workers[0].handle(apply(vec![TestEvent { id: 3, time: Time(insertion_time) }]));
    frontier.request(Time(cutoff));
    let round = frontier.begin().unwrap();
    for worker in &mut workers {
        worker.handle(WorkerMessage::Fence { round, router: 0 });
    }
    let mut permission = None;
    let mut reported_minima = Vec::new();
    for _ in 0..worker_count {
        let (round, id, minimum) = reports.try_recv().unwrap();
        reported_minima.push(minimum);
        if let Some(time) = frontier.report(round, id, minimum) {
            permission = Some(time);
        }
    }
    // Delay resolution, then forwarding. Neither gap may allow publication.
    assert!(!workers[0].step(), "a reported worker must wait for round resolution");
    for worker in &mut workers {
        worker.handle(WorkerMessage::Resolve { round, prune: advance(*frontier.safe()) });
        assert!(!worker.step(), "forwarding must finish before replay resumes");
        assert!(worker.prune_step());
        assert!(!worker.prune_step());
    }
    assert!(publications.is_empty(), "forwarding must not publish reactions");
    assert!(workers[0].step());
    let outputs: Vec<_> = publications.try_iter().collect();
    let (rejected, rejections) = unbounded();
    let mut admitted = 0;
    for &(time, _) in &outputs {
        let batch = RouterBatch {
            inputs: prepare_inputs(vec![TestEvent { id: 1_000 + u128::from(time.0), time }]),
            completion: CompletionHandle::new(rejected.clone()),
        };
        if matches!(admit::<_, TestSnapshot>(batch, Some(time), &mut frontier), Some(RouterMessage::Apply(_))) {
            admitted += 1;
        }
    }
    let errors: Vec<_> = rejections.try_iter().collect();

    assert_eq!(outputs.last(), Some(&(initial_target, expected_sum)));
    assert!(
        errors.is_empty() && admitted == outputs.len(),
        "reported={reported_minima:?}, permission={permission:?}, outputs={outputs:?}, rejections={errors:?}"
    );

    // Once replay has finished, the next round must reach the requested cutoff.
    if let Some(round) = frontier.begin() {
        for worker in &mut workers {
            worker.handle(WorkerMessage::Fence { round, router: 0 });
        }
        for _ in 0..worker_count {
            let (round, id, minimum) = reports.try_recv().unwrap();
            assert_eq!(minimum, None);
            frontier.report(round, id, minimum);
        }
        for worker in &mut workers {
            worker.handle(WorkerMessage::Resolve { round, prune: advance(*frontier.safe()) });
            assert!(worker.prune_step());
            assert!(!worker.prune_step());
        }
    }
    assert_eq!(*frontier.safe(), Time(cutoff));
    assert!(publications.is_empty(), "completed replay must not be republished by forwarding");
}

#[rstest::rstest]
#[case::during_measurement(false, 0)]
#[case::after_decision_before_delivery(true, 80)]
fn insertion_after_report_waits_for_resolution(#[case] decide_first: bool, #[case] expected_safe: u64) {
    let cutoff = Time(80);
    let target = Time(100);
    let expected_sum = 3;

    let messages = never();
    let registrations = never();
    let (reported, reports) = unbounded();
    let (published, publications) = unbounded();
    let mut worker = TestWorker::new(
        &messages,
        &registrations,
        CheckpointStorageConfig { checkpoints: CheckpointConfig { interval: 100 } },
        Publish { outputs: published },
        Some(Coordination {
            router_count: 1,
            report: Box::new(move |round, minimum| reported.send((round, minimum)).unwrap()),
            pruned: Box::new(|_| {}),
        }),
    );
    worker.handle(apply(vec![TestEvent { id: 1, time: Time(10) }, TestEvent { id: 2, time: target }]));
    worker.handle(WorkerMessage::Advance(advance(target)));
    assert!(worker.step());
    publications.try_iter().for_each(drop);
    let mut frontier = Frontier::new(1);
    frontier.request(cutoff);
    let round = frontier.begin().unwrap();
    worker.handle(WorkerMessage::Fence { round, router: 0 });
    let (_, minimum) = reports.try_recv().unwrap();
    assert_eq!(minimum, None);

    if decide_first {
        frontier.report(round, 0, minimum);
    }
    let (rejected, rejections) = unbounded();
    let batch = RouterBatch {
        inputs: prepare_inputs(vec![TestEvent { id: 3, time: Time(90) }]),
        completion: CompletionHandle::new(rejected.clone()),
    };
    let Some(RouterMessage::Apply(batch)) = admit::<_, TestSnapshot>(batch, None, &mut frontier) else { panic!("admissible insertion") };
    worker.handle(route(batch));
    assert!(!worker.step());
    let (response, queried) = unbounded();
    worker.handle(WorkerMessage::SnapshotQuery(crate::SnapshotQuery { time: target, snapshot_ids: vec![1], response }));
    assert_eq!(queried.try_recv().unwrap()[0].sum, expected_sum);
    assert!(publications.is_empty(), "queries during the barrier must remain effect-free");
    if !decide_first {
        frontier.report(round, 0, minimum);
    }
    worker.handle(WorkerMessage::Resolve { round, prune: advance(*frontier.safe()) });
    assert_eq!(worker.prune_step(), decide_first);
    assert!(!worker.prune_step());
    assert!(publications.is_empty());
    assert!(worker.step(), "an unchanged horizon must also release replay");
    let outputs: Vec<_> = publications.try_iter().collect();
    for &(time, _) in &outputs {
        let batch = RouterBatch {
            inputs: prepare_inputs(vec![TestEvent { id: 1_000 + u128::from(time.0), time }]),
            completion: CompletionHandle::new(rejected.clone()),
        };
        assert!(admit::<_, TestSnapshot>(batch, Some(time), &mut frontier).is_some());
    }

    assert_eq!(*frontier.safe(), Time(expected_safe));
    assert_eq!(outputs.last(), Some(&(target, expected_sum)));
    assert!(rejections.is_empty());
}
