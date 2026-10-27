//! Shared simulator, monitor-only control and validation; no terminal or process transport.
use super::{backend::Backend, control, metrics::Metrics, reference, Failure, Options};
use crate::definition::{Example, Time};
use contime_monitor::{WorkerMonitor, WorkerReading};
use crossbeam_channel::unbounded;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub trait Observer<M: Example> {
    fn should_stop(&mut self) -> Result<bool, Failure> {
        Ok(false)
    }
    fn interval(&self) -> Duration {
        Duration::from_millis(50)
    }
    fn report(
        &mut self,
        metrics: &Metrics,
        reading: Option<&Arc<WorkerReading<Time>>>,
        expected: &[M::State],
        verified: &[M::State],
        done: bool,
    ) -> Result<(), Failure>;
}
#[derive(Debug)]
pub struct Summary {
    pub events: u64,
    pub applications: u64,
    pub collections: u64,
    pub completed: u64,
    pub elapsed: Duration,
    pub target_rate: u64,
    pub queries: u64,
    pub reversed: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Forward,
    ReverseGroups,
}
struct Validation<S> {
    #[allow(clippy::vec_box)] // ConTime's existing query transport owns boxed snapshots.
    states: crossbeam_channel::Receiver<Vec<Box<S>>>,
    actual: Vec<S>,
    seen: Vec<bool>,
    deadline: Instant,
}
fn begin_validation<M: Example, B: Backend<M>>(contime: &B, count: usize, time: Time) -> Result<Validation<M::State>, Failure> {
    let (sender, states) = unbounded();
    contime.query(time, count, sender)?;
    Ok(Validation {
        states,
        actual: vec![M::State::default(); count],
        seen: vec![false; count],
        deadline: Instant::now() + Duration::from_secs(5),
    })
}
fn poll_validation<M: Example>(pending: &mut Validation<M::State>, expected: &[M::State]) -> Result<bool, Failure> {
    if Instant::now() >= pending.deadline {
        return Err("Snapshot validation timed out".into());
    }
    loop {
        match pending.states.try_recv() {
            Ok(states) => {
                reference::receive::<M>(states, &mut pending.actual, &mut pending.seen)?;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => return Ok(false),
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                reference::verify::<M>(&pending.actual, expected)?;
                return Ok(true);
            }
        }
    }
}
fn validate<M: Example, B: Backend<M>>(contime: &B, expected: &[M::State], time: Time) -> Result<Vec<M::State>, Failure> {
    let mut pending = begin_validation::<M, B>(contime, expected.len(), time)?;
    while !poll_validation::<M>(&mut pending, expected)? {
        if !contime.process()? {
            std::thread::sleep(Duration::from_micros(100));
        }
        contime.check_rejections()?;
    }
    Ok(pending.actual)
}
pub fn drive<M, G, It, B, O>(
    contime: &B,
    options: &Options,
    generate: &G,
    delivery: Delivery,
    cancelled: &AtomicBool,
    observer: &mut O,
) -> Result<Summary, Failure>
where
    M: Example,
    B: Backend<M>,
    O: Observer<M>,
    G: Fn(Options, u64, u64, Time) -> It,
    It: Iterator<Item = M::Event>,
{
    options.validate()?;
    let clock = Instant::now();
    let mut controller = control::Controller::new(options.rate, options.targets, options.maximum);
    if let Some(limit) = options.source_limit {
        controller.limit_sources(limit);
    }
    if delivery == Delivery::ReverseGroups {
        controller.reverse_groups();
    }
    let mut collector = WorkerMonitor::attach(contime);
    let mut latest = None;
    let mut collecting = None;
    let mut collections = 0;
    let mut next_status = Duration::ZERO;
    let mut next_draw = Duration::ZERO;
    let mut expected = vec![M::State::default(); options.snapshots];
    let mut verified = expected.clone();
    let mut next_validation = Duration::ZERO;
    let mut validation = None::<Validation<M::State>>;
    let mut queries = 0_u64;
    let mut reversed = 0_u64;
    let mut ids = Vec::new();
    let mut applications = 0_u64;
    let mut last_time = Time(0);
    let mut stopping = false;
    let mut stop_started = None;
    loop {
        let progressed = contime.process()?;
        let now = clock.elapsed();
        stopping |= cancelled.load(Ordering::Relaxed);
        stopping |= options.duration.is_some_and(|duration| now >= duration);
        stopping |= observer.should_stop()?;
        if stopping {
            controller.stop(now);
            let began = stop_started.get_or_insert_with(Instant::now);
            if began.elapsed() > Duration::from_secs(10) {
                return Err("Final processing confirmation timed out".into());
            }
            next_status = next_status.min(now);
        }
        contime.check_rejections()?;
        if let Some(pending) = validation.as_mut() {
            if poll_validation::<M>(pending, &expected)? {
                verified = validation.take().expect("completed validation exists").actual;
                next_validation = now + Duration::from_secs_f64(1.0 / options.query_hz as f64);
            }
        }
        // Freeze the canonical prefix/advancement until validation returns, while
        // monitoring and keyboard input continue on every loop turn.
        let available = if validation.is_some() || !contime.can_submit() { 0 } else { controller.allowance(now) };
        let count = if available < controller.minimum_submission() { 0 } else { available };
        if count > 0 {
            let time = Time((now.as_nanos() as u64).max(last_time.0 + count));
            let mut events = generate(options.clone(), controller.produced(), count, time).collect::<Vec<_>>();
            if events.len() as u64 != count {
                return Err("Simulator yielded the wrong event count".into());
            }
            let new_applications = reference::record::<M>(&mut events, &mut expected, options.targets, &mut ids)?;
            if delivery == Delivery::ReverseGroups {
                reversed += count.saturating_sub(1);
                events.reverse();
            }
            applications += new_applications;
            contime.submit(events)?;
            contime.advance(time)?;
            controller.submitted(count, new_applications, time, now)?;
            last_time = time;
            if controller.waiting() {
                next_status = now;
            }
        }
        if let Some(started) = collecting {
            if let Some(reading) = collector.poll(now)? {
                let completed_window = controller.observe(&reading)?;
                latest = Some(reading);
                if completed_window && (M::hash(&M::State::default()).is_some()) && now >= next_validation && !stopping {
                    validation = Some(begin_validation::<M, B>(contime, expected.len(), last_time)?);
                    queries += 1;
                }
                collections += 1;
                collecting = None;
            } else if now.saturating_sub(started) > Duration::from_secs(2) {
                return Err("Worker collection timed out".into());
            }
        }
        if stopping && controller.outstanding() == 0 && validation.is_none() {
            collector.cancel();
            let elapsed = clock.elapsed();
            verified = validate::<M, B>(contime, &expected, last_time)?;
            let summary = Summary {
                events: controller.produced(),
                completed: controller.completed(),
                applications,
                collections,
                elapsed,
                target_rate: controller.rate(),
                queries: queries + 1,
                reversed,
            };
            observer.report(
                &Metrics {
                    target_rate: summary.target_rate,
                    produced: summary.events,
                    completed: summary.completed,
                    outstanding: 0,
                    elapsed: summary.elapsed,
                    reversed: (delivery == Delivery::ReverseGroups).then_some(summary.reversed),
                    queries: summary.queries,
                },
                latest.as_ref(),
                &expected,
                &verified,
                true,
            )?;
            return Ok(summary);
        }
        if collecting.is_none() && now >= next_status {
            collector.begin(now)?;
            collecting = Some(now);
            next_status = now + if controller.waiting() { Duration::from_millis(2) } else { Duration::from_millis(50) };
        }
        // An unchanging monitor must not permit unlimited production or hang a run.
        // Numeric pacing can legitimately take longer than a Maximum window.
        if options.maximum && controller.window_started().is_some_and(|started| now.saturating_sub(started) > Duration::from_secs(10)) {
            return Err("Production window made no confirmed completion within ten seconds".into());
        }
        let metrics = Metrics {
            target_rate: controller.rate(),
            produced: controller.produced(),
            completed: controller.completed(),
            outstanding: controller.outstanding(),
            elapsed: now,
            reversed: (delivery == Delivery::ReverseGroups).then_some(reversed),
            queries,
        };
        if now >= next_draw {
            observer.report(&metrics, latest.as_ref(), &expected, &verified, false)?;
            next_draw = now + observer.interval();
        }
        // No deliberate wait in the producer hot path. Yield briefly only while
        // paced or waiting for the next worker observation.
        if count == 0 && !progressed {
            std::thread::sleep(Duration::from_micros(100));
        }
    }
}
