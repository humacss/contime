//! Fixed-work scaling benchmark. This is not part of the published workspace.
use contime::checkpoints::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, CheckpointConfig, Event, EventBatch, Snapshot, Timestamp};
use contime::{ConTime, ConTimeConfig, Input};
use contime::contime_api::{AdvanceOutput, ApplyOutput};
use contime::contime_router::{AdvanceWorkerOutput, RouteOutput, WorkerOutput};
use contime::contime_runtime::Worker;
use contime::contime_runtime::{CompletionHandle, Route, RouterMessage, RouterProcess, SharedEvent, WorkerMessage, WorkerProcess};
use crossbeam_channel::{unbounded, Sender};
use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Time(i64);
impl Timestamp for Time {
    fn previous(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}
impl contime::AdvanceTime for Time {
    fn saturating_sub(&self, rhs: &Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}
#[derive(Clone)]
struct Record {
    id: u128,
    history: u128,
    time: Time,
    rounds: usize,
}
impl Event for Record {
    type Time = Time;
    fn time(&self) -> Time {
        self.time
    }
}
impl Input for Record {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(self.history)
    }
}
#[derive(Clone, Default, Debug)]
struct State {
    id: u128,
    time: Time,
    hash: u64,
    count: usize,
}
fn response_channel(submission: usize, pool: usize) -> usize {
    submission % pool
}
fn calculate(mut hash: u64, event: &Record) -> u64 {
    hash = hash.wrapping_add(event.id as u64).rotate_left(13).wrapping_mul(0x9e3779b185ebca87);
    for _ in 0..event.rounds {
        hash = hash.rotate_left(13).wrapping_mul(0x9e3779b185ebca87).wrapping_add(0x517cc1b727220a95);
    }
    black_box(hash)
}
impl Snapshot for State {
    type Time = Time;
    fn time(&self) -> &Time {
        &self.time
    }
    fn set_time(&mut self, time: Time) {
        self.time = time;
    }
}
impl ApplyEvents<Record> for State {
    fn create(id: u128, _: &Record) -> Self {
        mark_start();
        Self { id, ..Self::default() }
    }
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, Record>) {
        for event in batch.events {
            self.hash = calculate(self.hash, event);
            self.count += 1;
        }
    }
}
#[derive(Clone)]
struct Observe {
    final_time: Time,
    done: Sender<Receipt>,
    histories: usize,
    completed: usize,
    calls: usize,
}
impl ApplyWrapper<State, Record> for Observe {
    fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, Time, Record>, inner: &mut ApplyInner<'_, State>) {
        self.calls += 1;
        let last = batch.time == self.final_time;
        inner.apply_event_batch(batch);
        if last {
            self.completed += 1;
            if self.completed == self.histories {
                let (cpu, started) = WORKER_START.with(|s| s.get().unwrap());
                self.done
                    .send(Receipt::Worker { cpu: thread_cpu() - cpu, span: started.elapsed().as_secs_f64(), calls: self.calls })
                    .unwrap();
            }
            self.done.send(Receipt::History(inner.snapshot().clone())).unwrap();
        }
    }
}
// Measure all process threads, so channel/admission/router CPU is included too.
fn cpu_clock(clock: libc::clockid_t) -> f64 {
    let mut value = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: the OS initializes the valid timespec pointer on successful return.
    let result = unsafe { libc::clock_gettime(clock, value.as_mut_ptr()) };
    assert_eq!(result, 0);
    let value = unsafe { value.assume_init() };
    value.tv_sec as f64 + value.tv_nsec as f64 * 1e-9
}
fn process_cpu() -> f64 {
    cpu_clock(libc::CLOCK_PROCESS_CPUTIME_ID)
}
fn thread_cpu() -> f64 {
    cpu_clock(libc::CLOCK_THREAD_CPUTIME_ID)
}
thread_local! { static WORKER_START: std::cell::Cell<Option<(f64, Instant)>> = const { std::cell::Cell::new(None) }; }
fn mark_start() {
    WORKER_START.with(|start| {
        if start.get().is_none() {
            start.set(Some((thread_cpu(), Instant::now())));
        }
    });
}
enum Receipt {
    History(State),
    Worker { cpu: f64, span: f64, calls: usize },
}
#[derive(Clone, Copy)]
struct Settings {
    workers: usize,
    histories: usize,
    total: usize,
    batch: usize,
    rounds: usize,
    response_channels: usize,
}
impl Settings {
    fn target(self) -> Time {
        Time((self.total / self.histories) as i64)
    }
    fn batches(self) -> Vec<Vec<Record>> {
        (0..self.total)
            .step_by(self.batch)
            .map(|first| {
                (first..(first + self.batch).min(self.total))
                    .map(|i| Record {
                        id: i as u128 + 1,
                        history: (i % self.histories) as u128,
                        time: Time((i / self.histories + 1) as i64),
                        rounds: self.rounds,
                    })
                    .collect()
            })
            .collect()
    }
}
fn verify(actual: Vec<State>, expected: &[State]) {
    let mut seen = vec![false; expected.len()];
    for state in actual {
        let i = state.id as usize;
        assert!(!seen[i], "duplicate completion for history {i}");
        seen[i] = true;
        assert_eq!((state.hash, state.count), (expected[i].hash, expected[i].count));
    }
    assert!(seen.into_iter().all(|x| x));
}
fn run(mode: &str, settings: Settings, expected: &[State]) -> (f64, f64, f64, f64, f64, usize) {
    let (done, receive) = unbounded();
    let wrapper = Observe { final_time: settings.target(), done, histories: settings.histories / settings.workers, completed: 0, calls: 0 };
    let batches = settings.batches();
    let checkpoints = CheckpointConfig { interval: 100 };
    let channels: Vec<_> = (0..settings.response_channels).map(|_| unbounded()).collect();
    let responses: Vec<_> = channels.iter().map(|(s, _)| s.clone()).collect();
    let mut native = None;
    let mut runtime = None;
    let mut direct = Vec::new();
    let mut threads = Vec::new();
    let barrier = Arc::new(Barrier::new(settings.workers + 1));
    if mode == "full" {
        let pipeline = ConTime::start(
            ConTimeConfig {
                router_count: 1,
                worker_count: settings.workers,
                placement: Default::default(),
                pruning_interval: Duration::from_secs(3600),
                history_retention: Time(i64::MAX),
                checkpoints,
            },
            wrapper.clone(),
        )
        .unwrap();
        pipeline.advance_to(settings.target()).unwrap();
        pipeline.wait_until_idle(Duration::from_secs(30)).unwrap(); // startup only, never timed
        native = Some(pipeline);
    } else if mode == "routed" {
        let topology = contime::contime_runtime::ThreadTopology::start(
            vec![RouterProcess::<Record, State>::new(Default::default())],
            (0..settings.workers).map(|_| WorkerProcess::new(checkpoints, wrapper.clone())).collect(),
        )
        .unwrap();
        topology.input().send(RouterMessage::advance(settings.target(), unbounded().0)).ok().unwrap();
        runtime = Some(topology);
    } else if mode == "workers" || mode == "preloaded" {
        for _ in 0..settings.workers {
            let (sender, receiver) = unbounded();
            sender.send(WorkerMessage::advance(settings.target(), unbounded().0)).ok().unwrap();
            let worker = WorkerProcess::new(checkpoints, wrapper.clone());
            let gate = barrier.clone();
            let preloaded = mode == "preloaded";
            threads.push(std::thread::spawn(move || {
                if preloaded {
                    gate.wait();
                }
                worker.run(receiver).unwrap();
            }));
            direct.push(sender);
        }
    }
    if mode == "compute" {
        for worker in 0..settings.workers {
            let barrier = barrier.clone();
            let done = wrapper.done.clone();
            let work: Vec<_> = batches.iter().flatten().filter(|e| e.history as usize % settings.workers == worker).cloned().collect();
            threads.push(std::thread::spawn(move || {
                let mut states = vec![State::default(); settings.histories];
                barrier.wait();
                let cpu = thread_cpu();
                let started = Instant::now();
                for event in work {
                    let state = &mut states[event.history as usize];
                    state.id = event.history;
                    state.hash = calculate(state.hash, &event);
                    state.count += 1;
                }
                done.send(Receipt::Worker { cpu: thread_cpu() - cpu, span: started.elapsed().as_secs_f64(), calls: 0 }).unwrap();
                for state in states.into_iter().filter(|s| s.count != 0) {
                    done.send(Receipt::History(state)).unwrap();
                }
            }));
        }
    }
    let batches = if mode == "preloaded" {
        for (submission, batch) in batches.into_iter().enumerate() {
            let reject = &responses[response_channel(submission, responses.len())];
            dispatch(batch, &direct, settings.workers, reject);
        }
        Vec::new()
    } else {
        batches
    };
    let batches = if mode == "compute" {
        drop(batches);
        Vec::new()
    } else {
        batches
    };
    let cpu_start = process_cpu();
    let start = Instant::now();
    if mode == "compute" || mode == "preloaded" {
        barrier.wait();
    }
    for (submission, batch) in batches.into_iter().enumerate() {
        let reject = &responses[response_channel(submission, responses.len())];
        match mode {
            "full" => native.as_ref().unwrap().send(batch, reject.clone()).unwrap(),
            "routed" => {
                let shared = batch.into_iter().map(SharedEvent::from).collect();
                runtime.as_ref().unwrap().input().send(RouterMessage::create(shared, reject.clone())).ok().unwrap();
            }
            "workers" => dispatch(batch, &direct, settings.workers, reject),
            "compute" => (),
            _ => panic!("unknown mode {mode}"),
        }
    }
    let submitted = start.elapsed().as_secs_f64();
    let mut states = Vec::new();
    let mut worker_cpu = 0.;
    let mut span: f64 = 0.;
    let mut calls = 0;
    let mut reporting_workers = 0;
    while states.len() < settings.histories {
        match receive.recv_timeout(Duration::from_secs(120)).expect("application completion") {
            Receipt::History(state) => states.push(state),
            Receipt::Worker { cpu, span: worker_span, calls: n } => {
                reporting_workers += 1;
                worker_cpu += cpu;
                span = span.max(worker_span);
                calls += n;
            }
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let cpu = process_cpu() - cpu_start;
    verify(states, expected);
    assert_eq!(reporting_workers, settings.workers);
    if mode != "compute" {
        assert_eq!(calls, settings.total, "live application must visit every event exactly once");
    }
    drop(responses);
    drop(direct);
    if let Some(pipeline) = native {
        let actual = pipeline.query_at(settings.target(), 0..settings.histories as u128).unwrap();
        verify(actual.into_iter().map(|s| *s).collect(), expected);
        let report = pipeline.shutdown();
        assert!(report.workers.iter().all(|r| *r == contime::contime_runtime::ThreadOutcome::Completed));
    }
    if let Some(topology) = runtime {
        topology.shutdown();
    }
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(channels.iter().map(|(_, r)| r.try_iter().count()).sum::<usize>(), 0);
    (elapsed, cpu, submitted, worker_cpu, span, calls)
}
fn dispatch(
    batch: Vec<Record>,
    direct: &[Sender<WorkerMessage<Record, State>>],
    workers: usize,
    reject: &Sender<contime::RejectionMessage<contime::RejectionReason>>,
) {
    let mut routes: Vec<Vec<_>> = (0..workers).map(|_| Vec::new()).collect();
    for event in batch {
        let id = event.history;
        routes[id as usize % workers].push(Route::create(id, SharedEvent::from(event)));
    }
    for (worker, routes) in routes.into_iter().enumerate() {
        if !routes.is_empty() {
            direct[worker].send(WorkerMessage::create(routes, CompletionHandle::new(reject.clone()))).ok().unwrap();
        }
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let parse = |i: usize, default: usize| args.get(i).map_or(default, |s| s.parse().unwrap());
    let total = parse(1, 262144);
    let rounds = parse(2, 0);
    let batch = parse(3, 1024);
    let repeats = parse(4, 5);
    let modes = args.get(5).map_or("compute,preloaded,workers,routed,full", String::as_str);
    let histories = 64;
    assert!(total > 0 && batch > 0 && repeats > 0);
    assert_eq!(total % histories, 0, "event count must be a multiple of 64");
    assert!(modes.split(',').all(|m| ["compute", "preloaded", "workers", "routed", "full"].contains(&m)));
    let response_pools: Vec<usize> = args.get(6).map_or("1", String::as_str).split(',').map(|n| n.parse().unwrap()).collect();
    assert!(response_pools.iter().all(|n| *n > 0));
    let response_channels = 1;
    let settings = Settings { total, rounds, batch, workers: 1, histories, response_channels };
    let mut expected = vec![State::default(); histories];
    for event in settings.batches().into_iter().flatten() {
        let state = &mut expected[event.history as usize];
        state.hash = calculate(state.hash, &event);
        state.count += 1;
    }
    println!("mode,workers,total,histories,rounds,batch,response_channels,repeat,wall_ms,process_cpu_ms,submit_ms,worker_cpu_ms,max_worker_span_ms,application_calls");
    for repeat in 0..repeats {
        // Rotate order to avoid systematically favouring one worker count.
        for j in 0..3 {
            let workers = [1, 2, 4][(j + repeat) % 3];
            for mode in modes.split(',') {
                for &response_channels in &response_pools {
                    let (wall, cpu, submit, worker_cpu, span, calls) =
                        run(mode, Settings { workers, response_channels, ..settings }, &expected);
                    println!(
                        "{mode},{workers},{total},{histories},{rounds},{batch},{response_channels},{repeat},{:.3},{:.3},{:.3},{:.3},{:.3},{calls}",
                        wall * 1000., cpu * 1000., submit * 1000., worker_cpu * 1000., span * 1000.
                    );
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_layer_completes_identical_independent_histories() {
        for batch in [1, 127, 1024] {
            for response_channels in [1, 64] {
                let settings = Settings { total: 8192, histories: 64, workers: 2, batch, rounds: 3, response_channels };
                let mut expected = vec![State::default(); settings.histories];
                for event in settings.batches().into_iter().flatten() {
                    let state = &mut expected[event.history as usize];
                    state.hash = calculate(state.hash, &event);
                    state.count += 1;
                }
                for mode in ["compute", "preloaded", "workers", "routed", "full"] {
                    run(mode, settings, &expected);
                }
            }
        }
    }
    #[test]
    fn aligned_batches_use_the_entire_response_pool() {
        for batch in [64, 1000, 1024] {
            let settings = Settings { total: batch * 64, histories: 64, workers: 2, batch, rounds: 0, response_channels: 64 };
            let selected: Vec<_> = settings.batches().iter().enumerate().map(|(i, _)| response_channel(i, 64)).collect();
            assert_eq!(selected, (0..64).collect::<Vec<_>>());
        }
        assert_eq!(response_channel(64, 64), 0);
        assert_eq!(response_channel(100, 1), 0);
    }
    #[test]
    fn fingerprint_detects_reordering() {
        for rounds in [0, 1, 3] {
            let a = Record { id: 1, history: 0, time: Time(1), rounds };
            let b = Record { id: 2, history: 0, time: Time(2), rounds };
            assert_ne!(calculate(calculate(0, &a), &b), calculate(calculate(0, &b), &a));
        }
    }
}
