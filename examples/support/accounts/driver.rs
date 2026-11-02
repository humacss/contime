//! Fixed, repeatable competing-payment scenario. No settlement or external effects.
use super::{reference::Reference, report, Config};
use crate::{
    accounts_example::{
        model::{Account, Transaction},
        setup::AccountExample,
    },
    definition::Time,
    support::monitored::backend::Backend,
};
use contime::contime_monitor::WorkerMonitor;
use crossbeam_channel::{Receiver, TryRecvError};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    time::Duration,
};
type Failure = Box<dyn std::error::Error>;
#[path = "../../cases/account_balances/generation.rs"]
mod generation;
struct Seal {
    time: Time,
    inserted: u64,
    started: Duration,
    final_query: bool,
}
pub struct Driver {
    pub config: Config,
    pub now: Duration,
    pub done: bool,
    pub progressed: bool,
    pub submitted: u64,
    next: u64,
    waiting: BTreeMap<(u64, u64), Transaction>,
    recent: VecDeque<report::Delivery>,
    seen: HashMap<u128, Time>,
    reference: Reference,
    arrival: Reference,
    seal: Option<Seal>,
    #[allow(clippy::vec_box)] // Backend query transport owns boxed snapshots.
    receiver: Option<Receiver<Vec<Box<Account>>>>,
    collecting: bool,
    next_query: u64,
    stopped: Option<(Time, Duration)>,
    target: Time,
    view: report::Report,
}
/// One eight-second round: deposit, two competing requests, two unchanged retries, refund.
fn input(index: u64) -> (Transaction, u64, bool) {
    let round = index / 6;
    let (offset, delay, slot, amount, retry) = match index % 6 {
        0 => (100, 200, 1, 100, false),
        1 => (1000, 1800, 2, -80, false),
        2 => (1000, 4100, 2, -80, true),
        3 => (2000, 200, 3, -60, false),
        4 => (2000, 600, 3, -60, true),
        _ => (4000, 500, 4, 20, false),
    };
    let at = round.saturating_mul(8000).saturating_add(offset);
    (generation::transaction(u128::from(round) * 4 + slot, Time(at.saturating_mul(1_000_000).saturating_add(1)), amount), delay, retry)
}
impl Driver {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            now: Duration::ZERO,
            done: false,
            progressed: false,
            submitted: 0,
            next: 0,
            waiting: BTreeMap::new(),
            recent: VecDeque::new(),
            seen: HashMap::new(),
            reference: Reference::new(),
            arrival: Reference::new(),
            seal: None,
            receiver: None,
            collecting: false,
            next_query: 0,
            stopped: None,
            target: Time(1),
            view: report::Report::default(),
        }
    }
    pub fn step<B: Backend<AccountExample>>(
        &mut self,
        backend: &B,
        monitor: &mut WorkerMonitor<'_, Time, B>,
        elapsed: Duration,
        stop: bool,
    ) -> Result<bool, Failure> {
        if elapsed < self.now {
            return Err("Account clock went backwards".into());
        }
        self.now = elapsed;
        if self.done {
            return Ok(true);
        }
        let now = u64::try_from(elapsed.as_nanos())?.checked_add(1).ok_or("Account clock exhausted")?;
        self.progressed = backend.process()?;
        backend.check_rejections()?;
        if stop || self.config.duration.is_some_and(|duration| elapsed >= duration) {
            let (at, _) = *self.stopped.get_or_insert((Time(now), elapsed));
            self.waiting.retain(|_, event| event.time <= at);
        }
        if self.stopped.is_none() {
            for _ in 0..64 {
                let (event, delay, _) = input(self.next);
                // Delivery, rather than source order, determines the network queue.
                if event.time.0 > now.saturating_add(3_500_000_000) {
                    break;
                }
                self.waiting.insert((event.time.0.saturating_add(delay * 1_000_000), self.next), event);
                self.next = self.next.checked_add(1).ok_or("Account sequence exhausted")?;
            }
        }
        if self.waiting.len() > 8192 {
            return Err("Account network capacity exceeded".into());
        }
        if self.stopped.is_some_and(|(_, at)| elapsed.saturating_sub(at) > Duration::from_secs(15)) {
            return Err("Account drain timed out".into());
        }
        if self.seal.is_none() && backend.can_submit() {
            let limit = if self.stopped.is_some() { u64::MAX } else { now };
            let keys: Vec<_> = self.waiting.range(..=(limit, u64::MAX)).take(64).map(|(key, _)| *key).collect();
            let mut events = Vec::new();
            for key in keys {
                let event = self.waiting.remove(&key).unwrap();
                let duplicate = self.seen.contains_key(&event.id);
                if !duplicate {
                    self.seen.insert(event.id, event.time);
                    self.submitted += 1;
                    self.reference.insert(event.clone());
                    let mut arrived = event.clone();
                    arrived.time = Time(now);
                    self.arrival.insert_arrival(arrived, self.submitted);
                } else {
                    self.view.retries += 1;
                }
                self.recent.push_back(delivery(key, &event, Some(now), duplicate));
                while self.recent.len() > 64 {
                    self.recent.pop_front();
                }
                events.push(event);
            }
            // Retries reach ConTime unchanged: deduplication is not performed by this producer.
            if !events.is_empty() {
                backend.submit(events)?;
            }
            if now >= self.next_query || self.stopped.is_some() {
                let oldest = self
                    .waiting
                    .values()
                    .map(|event| event.time.0)
                    .chain(self.stopped.is_none().then(|| input(self.next).0.time.0))
                    .min()
                    .unwrap_or(now);
                self.target = self.target.max(Time(now.min(oldest.saturating_add(self.config.retention_ms * 1_000_000).saturating_sub(1))));
                backend.advance(self.target)?;
                self.seal = Some(Seal {
                    time: self.target,
                    inserted: self.submitted,
                    started: elapsed,
                    final_query: self.stopped.is_some() && self.waiting.is_empty(),
                });
            }
        }
        if self.seal.is_some() && self.receiver.is_none() && !self.collecting {
            monitor.begin(elapsed)?;
            self.collecting = true;
        }
        if self.collecting {
            if let Some(reading) = monitor.poll(elapsed)? {
                self.collecting = false;
                let seal = self.seal.as_ref().unwrap();
                let inserted = reading
                    .workers
                    .iter()
                    .try_fold(0u64, |sum, w| w.counters.filter(|c| !c.saturated).and_then(|c| sum.checked_add(c.inserted_applications)));
                if inserted == Some(seal.inserted)
                    && reading.workers.iter().all(|w| {
                        w.due_histories == 0
                            && w.pending_prune_passes == 0
                            && w.awaiting_resolution.is_none()
                            && w.processing_target >= seal.time
                    })
                {
                    if seal.inserted == 0 {
                        self.finish_query(Account { account_id: 1, ..Default::default() })?;
                    } else {
                        let (send, receive) = crossbeam_channel::unbounded();
                        backend.query(seal.time, 1, send)?;
                        self.receiver = Some(receive);
                    }
                }
            }
        }
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(mut states) => {
                    if states.len() != 1 {
                        return Err("Expected one queried account".into());
                    }
                    self.finish_query(*states.remove(0))?;
                    self.receiver = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return Err("Account query disconnected".into()),
            }
        }
        if self.seal.as_ref().is_some_and(|seal| elapsed.saturating_sub(seal.started) > Duration::from_secs(10)) {
            return Err("Account query confirmation timed out".into());
        }
        Ok(self.done)
    }
    fn finish_query(&mut self, state: Account) -> Result<(), Failure> {
        let seal = self.seal.take().unwrap();
        let expected = self.reference.at(seal.time);
        if state.balance != expected.balance || state.events != expected.events || state.decisions != expected.decisions {
            return Err("Account query disagrees with chronological reference".into());
        }
        self.view.balance = state.balance;
        let arrival = self.arrival.at(seal.time);
        self.view.arrival_balance = arrival.balance;
        self.view.reconciled = state.decisions.iter().map(decision).collect();
        self.view.arrival_order = arrival.decisions.iter().map(decision).collect();
        self.view.query_ms = millis(seal.time.0);
        self.view.revision += 1;
        self.next_query = (self.now.as_nanos() as u64).saturating_add(1_000_000_000 / self.config.query_hz);
        let before = Time(seal.time.0.saturating_sub(self.config.retention_ms * 1_000_000));
        self.reference.compact(before);
        self.arrival.compact(before);
        self.seen.retain(|_, time| *time >= before);
        self.done = seal.final_query;
        Ok(())
    }
    pub fn report(&self) -> report::Report {
        let mut report = self.view.clone();
        report.clock_ms = self.now.as_millis() as u64;
        report.submitted = self.submitted;
        report.done = self.done;
        report.deliveries = self.recent.iter().cloned().collect();
        report.deliveries.extend(self.waiting.iter().take(64).map(|(&key, event)| delivery(key, event, None, false)));
        report
    }
}
fn millis(time: u64) -> u64 {
    time.saturating_sub(1) / 1_000_000
}
fn delivery(key: (u64, u64), event: &Transaction, arrived: Option<u64>, duplicate: bool) -> report::Delivery {
    report::Delivery {
        sequence: key.1,
        transaction: event.id as u64,
        action_ms: millis(event.time.0),
        due_ms: millis(key.0),
        arrived_ms: arrived.map(millis),
        amount: event.amount,
        duplicate,
    }
}
fn decision(d: &crate::accounts_example::model::Decision) -> report::Decision {
    report::Decision { id: d.id as u64, time_ms: millis(d.time.0), amount: d.amount, accepted: d.accepted, balance: d.balance }
}
#[cfg(all(test, feature = "manual"))]
#[path = "tests.rs"]
mod tests;
