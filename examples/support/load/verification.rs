//! Complete worker operations with independent verification between workload phases.
use crate::definition::{Example, Time};
use contime::checkpoints::CheckpointConfig;
use contime::checkpoints::{ApplyEvents, Event, Snapshot};
use contime::manual::{FrameReport, RawManualConTime, ManualConfig, StopReason};
use serde::Serialize;
use std::{cell::RefCell, collections::VecDeque, num::NonZeroUsize, rc::Rc};

pub(crate) type Instance<M> = RawManualConTime<<M as Example>::Event, <M as Example>::State>;
enum QueryResult<M: Example> {
    Snapshot(Vec<M::State>),
    RetainedTimes(Vec<Time>),
}

pub(crate) struct Workload<M: Example> {
    batch_size: usize,
    expected: VecDeque<M::State>,
    results: Rc<RefCell<VecDeque<QueryResult<M>>>>,
    expected_rejections: VecDeque<u128>,
    retention_check: Option<Time>,
    pub produced: M::State,
    pub queried: M::State,
    expected_hash: [u8; 32],
    pub verified: u64,
    pub duplicates: u64,
    pub rejected: u64,
    round: u64,
    retained: usize,
    horizon: Time,
    next_horizon: Option<Time>,
    deferred: Option<Vec<M::Event>>,
    stage: u8,
    pub(crate) failed: Option<String>,
    corrupt_next: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub steps: usize,
    pub elapsed_ms: f64,
    pub overrun: bool,
    pub pending: usize,
    pub busy: bool,
    pub completed: usize,
    pub verified: u64,
    pub duplicates: u64,
    pub rejected: u64,
    pub round: u64,
    pub retained: usize,
    pub horizon: u64,
    pub produced_count: u64,
    pub queried_count: u64,
    pub expected_hash: String,
    pub queried_hash: String,
    pub produced_hash: String,
}

impl<M: Example> Workload<M> {
    pub fn new(batch_size: usize) -> Result<(Instance<M>, Self), String> {
        if !(2..=8192).contains(&batch_size) || !batch_size.is_multiple_of(2) {
            return Err("Choose an even event count between 2 and 8192".into());
        }
        let capacity = NonZeroUsize::new(batch_size + 32).expect("validated workload is nonempty");
        let backend = RawManualConTime::new(ManualConfig { queue_capacity: capacity, checkpoints: CheckpointConfig { interval: 32 } }, ());
        Ok((
            backend,
            Self {
                batch_size,
                expected: VecDeque::new(),
                results: Rc::default(),
                expected_rejections: VecDeque::new(),
                retention_check: None,
                produced: M::State::default(),
                queried: M::State::default(),
                expected_hash: [0; 32],
                verified: 0,
                duplicates: 0,
                rejected: 0,
                round: 0,
                retained: 0,
                horizon: Time(0),
                next_horizon: None,
                deferred: None,
                stage: 0,
                failed: None,
                corrupt_next: false,
            },
        ))
    }
    pub fn inject_mismatch(&mut self) {
        self.corrupt_next = true;
    }
    pub fn enqueue_round(&mut self, backend: &mut Instance<M>) -> Result<(), String> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        if backend.queued() != 0 || self.stage != 0 {
            return Err("Finish the queued round first".into());
        }
        let base = self.produced.time().0;
        let events: Vec<_> =
            (1..=self.batch_size as u64).map(|offset| M::event(u128::from(base + offset), Time(base + offset), vec![1], 1)).collect();
        let half = events.len() / 2;
        let target = events.last().expect("validated batch has events").time();
        for event in events[half..].iter().rev() {
            Self::submit(backend, event.clone())?;
        }
        backend.advance_to(target).map_err(|e| e.to_string())?;
        let mut partial = if M::count(&self.produced) == 0 { M::State::create(1, &events[half]) } else { self.produced.clone() };
        if M::count(&self.produced) == 0 {
            self.produced = M::State::create(1, &events[0]);
        }
        for event in &events[half..] {
            M::record(&mut partial, event);
            partial.set_time(event.time());
        }
        self.query(backend, target, partial)?;
        for event in &events {
            M::record(&mut self.produced, event);
            self.produced.set_time(event.time());
        }
        self.deferred = Some(events);
        self.stage = 1;
        self.round += 1;
        Ok(())
    }
    fn submit(backend: &mut Instance<M>, event: M::Event) -> Result<(), String> {
        backend.apply([event]).map(|_| ()).map_err(|error| error.to_string())
    }
    fn query(&mut self, backend: &mut Instance<M>, target: Time, expected: M::State) -> Result<(), String> {
        let results = self.results.clone();
        backend
            .query_at(target, [1], move |states| {
                results.borrow_mut().push_back(QueryResult::Snapshot(states.into_iter().map(|state| *state).collect()))
            })
            .map_err(|e| e.to_string())?;
        self.expected.push_back(expected);
        Ok(())
    }
    pub fn collect(&mut self, backend: &mut Instance<M>, report: FrameReport) -> Result<Report, String> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        let rejections = backend.take_rejections();
        let mut completed = rejections.len();
        for rejection in rejections {
            if self.expected_rejections.pop_front() != Some(rejection.event_id)
                || rejection.reason != contime::RejectionReason::BeforeHistoryHorizon
            {
                let error = format!("Unexpected insertion rejection: {rejection:?}");
                self.failed = Some(error.clone());
                return Err(error);
            }
            self.rejected += 1;
        }
        while let Some(result) = { self.results.borrow_mut().pop_front() } {
            completed += 1;
            match result {
                QueryResult::Snapshot(mut states) => {
                    if states.len() != 1 {
                        let error = "Expected one retained snapshot".to_string();
                        self.failed = Some(error.clone());
                        return Err(error);
                    }
                    let snapshot = states.pop().expect("one snapshot checked above");
                    let expected = self.expected.pop_front().ok_or("missing query reference")?;
                    let mismatch = self.corrupt_next
                        || M::id(&snapshot) != M::id(&expected)
                        || M::count(&snapshot) != M::count(&expected)
                        || M::value(&snapshot) != M::value(&expected)
                        || M::hash(&snapshot) != M::hash(&expected)
                        || snapshot.time() != expected.time();
                    self.corrupt_next = false;
                    if mismatch {
                        let error = format!("Fatal hash/state mismatch in budget round {}", self.round);
                        self.failed = Some(error.clone());
                        return Err(error);
                    }
                    self.expected_hash = M::hash(&expected).unwrap_or_default();
                    self.queried = snapshot;
                    self.verified += 1;
                    if self.stage == 1 || self.stage == 2 {
                        self.retained += self.batch_size / 2;
                    }
                    if self.stage == 2 {
                        // This query verifies the duplicate left the full chain unchanged.
                        self.duplicates += 1;
                    }
                    if self.stage == 3 {
                        if let Some(horizon) = self.next_horizon.take() {
                            self.horizon = horizon;
                        }
                    }
                }
                QueryResult::RetainedTimes(times) => {
                    let horizon = self.retention_check.take().ok_or("missing retention reference")?;
                    let expected: Vec<_> = (horizon.0.max(1)..=self.produced.time().0).map(Time).collect();
                    if times != expected {
                        let error = "Fatal retained-event mismatch after pruning".to_string();
                        self.failed = Some(error.clone());
                        return Err(error);
                    }
                    self.retained = times.len();
                }
            }
        }
        // Yield to the host only between complete worker operations. Do not
        // deliver late events until live replay of the newer half has finished.
        if report.stopped == StopReason::NoReadyWork && backend.queued() == 0 {
            match self.stage {
                1 => {
                    let events = self.deferred.take().expect("first phase retains its events");
                    for event in events[..events.len() / 2].iter().rev() {
                        Self::submit(backend, event.clone())?;
                    }
                    Self::submit(backend, events[0].clone())?;
                    backend.advance_to(*self.produced.time()).map_err(|e| e.to_string())?;
                    self.query(backend, *self.produced.time(), self.produced.clone())?;
                    self.stage = 2;
                }
                2 => {
                    let horizon = Time(self.produced.time().0.saturating_sub(32));
                    if horizon > self.horizon {
                        backend.prune_before(horizon).map_err(|e| e.to_string())?;
                        self.next_horizon = Some(horizon);
                        let expired_id = u128::MAX - u128::from(self.produced.time().0);
                        self.expected_rejections.push_back(expired_id);
                        Self::submit(backend, M::event(expired_id, Time(horizon.0 - 1), vec![1], 99))?;
                    }
                    self.query(backend, *self.produced.time(), self.produced.clone())?;
                    let results = self.results.clone();
                    backend
                        .query_events_between(1, Time(0), Time(u64::MAX), move |events| {
                            results
                                .borrow_mut()
                                .push_back(QueryResult::RetainedTimes(events.into_iter().map(|event| event.time()).collect()));
                        })
                        .map_err(|e| e.to_string())?;
                    self.retention_check = Some(horizon.max(self.horizon));
                    self.stage = 3;
                }
                3 => {
                    if !self.expected.is_empty() || self.retention_check.is_some() || !self.expected_rejections.is_empty() {
                        let error = "Missing verification result at end of round".to_string();
                        self.failed = Some(error.clone());
                        return Err(error);
                    }
                    self.stage = 0;
                }
                _ => {}
            }
        }
        let hash = |bytes: [u8; 32]| blake3::Hash::from(bytes).to_hex().to_string();
        Ok(Report {
            steps: report.steps,
            elapsed_ms: report.elapsed.as_secs_f64() * 1000.0,
            overrun: report.overrun,
            pending: backend.queued(),
            busy: report.stopped != StopReason::NoReadyWork || self.stage != 0,
            completed,
            verified: self.verified,
            duplicates: self.duplicates,
            rejected: self.rejected,
            round: self.round,
            retained: self.retained,
            horizon: self.horizon.0,
            produced_count: M::count(&self.produced),
            queried_count: M::count(&self.queried),
            expected_hash: if M::hash(&self.queried).is_some() { hash(self.expected_hash) } else { state_digest::<M>(&self.queried) },
            queried_hash: state_digest::<M>(&self.queried),
            produced_hash: state_digest::<M>(&self.produced),
        })
    }
}

fn state_digest<M: Example>(state: &M::State) -> String {
    M::hash(state).map_or_else(
        || format!("count={} value={}", M::count(state), M::value(state)),
        |hash| blake3::Hash::from(hash).to_hex().to_string(),
    )
}
