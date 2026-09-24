//! Serializes admission with conservative, FIFO-fenced pruning rounds.
use std::sync::Arc;
use std::time::{Duration, Instant};

use contime_worker::AdvanceTime;
use crossbeam_channel::{Receiver, Sender};

use crate::frontier::Frontier;
use crate::{Advance, Input, RejectionMessage, RejectionReason, RouterBatch, RouterMessage};

pub(crate) fn run<I: Input, S>(
    input: Arc<Receiver<RouterMessage<I, S>>>,
    output: Sender<RouterMessage<I, S>>,
    controls: Vec<Sender<contime_router::Flush>>,
    registrations: Receiver<Sender<bool>>,
    retention: I::Time,
    workers: usize,
    pruning_interval: Duration,
) {
    let mut frontier = Frontier::new(workers);
    let mut target = I::Time::default();
    let (observed, observations) = crossbeam_channel::unbounded();
    let mut listeners: Vec<Sender<bool>> = Vec::new();
    let mut working = false;
    let mut next_round = Instant::now();
    let mut awaiting_marker = None;
    let mut resolving = None;
    let mut pruned = vec![I::Time::default(); workers];
    let mut completed_horizon = I::Time::default();
    let mut horizon_listeners: Vec<Sender<I::Time>> = Vec::new();
    loop {
        let pruning = frontier.requested() > frontier.safe();
        if pruning && !frontier.measuring() && resolving.is_none() && Instant::now() >= next_round {
            if !working {
                working = true;
                listeners.retain(|listener| listener.send(true).is_ok());
            }
            if let Some(round) = frontier.begin() {
                awaiting_marker = Some(round);
                if output.send(RouterMessage::Fence { round, observed: observed.clone() }).is_err() {
                    return;
                }
                next_round = Instant::now() + pruning_interval;
            }
        }
        let busy = pruning || resolving.is_some() || !input.is_empty() || !observations.is_empty();
        if working != busy {
            working = busy;
            listeners.retain(|listener| listener.send(working).is_ok());
        }
        let timer = if pruning && !frontier.measuring() && resolving.is_none() {
            crossbeam_channel::after(next_round.saturating_duration_since(Instant::now()))
        } else {
            crossbeam_channel::never()
        };
        let mut ready = crossbeam_channel::Select::new();
        let unresolved = crossbeam_channel::never();
        let resolution_index = ready.recv(resolving.as_ref().unwrap_or(&unresolved));
        let message_index = ready.recv(&input);
        let observed_index = ready.recv(&observations);
        let registration_index = ready.recv(&registrations);
        ready.recv(&timer);
        // Publish working before removing a queued message, so idle observers
        // cannot mistake a handoff for an empty pipeline.
        let selected = ready.ready();
        if selected == resolution_index {
            // Every routed completion owner has finished forwarding (or released
            // an unchanged horizon). Do not overlap measurement rounds.
            resolving = None;
            continue;
        }
        if selected == registration_index {
            match registrations.try_recv() {
                Ok(listener) => {
                    if listener.send(working).is_ok() {
                        listeners.push(listener);
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
            continue;
        }
        if !working {
            working = true;
            listeners.retain(|listener| listener.send(true).is_ok());
        }
        if selected == observed_index {
            if let Ok(round) = observations.try_recv() {
                if awaiting_marker == Some(round) {
                    awaiting_marker = None;
                    for (router, control) in controls.iter().enumerate() {
                        if control.send(contime_router::Flush { round, router }).is_err() {
                            return;
                        }
                    }
                }
            }
            continue;
        }
        if selected != message_index {
            continue;
        }
        let message = match input.try_recv() {
            Ok(message) => message,
            Err(crossbeam_channel::TryRecvError::Empty) => continue,
            Err(crossbeam_channel::TryRecvError::Disconnected) => return,
        };
        let forwarded = match message {
            RouterMessage::SubscribePrunedHorizon(listener) => {
                if listener.send(completed_horizon.clone()).is_ok() {
                    horizon_listeners.push(listener);
                }
                None
            }
            RouterMessage::Pruned { worker, horizon } => {
                if let Some(previous) = pruned.get_mut(worker) {
                    *previous = previous.clone().max(horizon);
                    let minimum = pruned.iter().min().expect("at least one worker");
                    if minimum > &completed_horizon {
                        completed_horizon = minimum.clone();
                        horizon_listeners.retain(|listener| listener.send(completed_horizon.clone()).is_ok());
                    }
                }
                None
            }
            RouterMessage::Apply(batch) => admit(batch, None, &mut frontier),
            RouterMessage::Internal { source, batch } => admit(batch, Some(source), &mut frontier),
            RouterMessage::Advance(mut advance) => {
                target = target.max(advance.time);
                frontier.request(target.saturating_sub(&retention));
                advance.time = target.clone();
                Some(RouterMessage::Advance(advance))
            }
            RouterMessage::Report { round, worker, minimum } => {
                let measuring = frontier.measuring();
                frontier.report(round, worker, minimum);
                if measuring && !frontier.measuring() {
                    let (completion, completed) = crossbeam_channel::unbounded();
                    resolving = Some(completed);
                    Some(RouterMessage::Resolve { round, prune: Advance { time: frontier.safe().clone(), completion } })
                } else {
                    None
                }
            }
            RouterMessage::Shutdown => return,
            other => Some(other),
        };
        if let Some(message) = forwarded {
            if output.send(message).is_err() {
                return;
            }
        }
    }
}

pub(crate) fn admit<I: Input, S>(
    mut batch: RouterBatch<I>,
    source: Option<I::Time>,
    frontier: &mut Frontier<I::Time>,
) -> Option<RouterMessage<I, S>> {
    batch.inputs.retain(|input| {
        let time = input.time();
        let reason = match &source {
            Some(source) if time < *source => Some(RejectionReason::BeforeSourceTime),
            Some(_) if time < *frontier.safe() => Some(RejectionReason::BeforeSafeTime),
            None if time < *frontier.requested() => Some(RejectionReason::BeforeHistoryHorizon),
            _ => None,
        };
        if let Some(reason) = reason {
            let _ = batch.completion.sender.send(RejectionMessage { event_id: input.event_id(), reason });
            false
        } else {
            frontier.admitted(&time);
            true
        }
    });
    (!batch.inputs.is_empty()).then_some(RouterMessage::Apply(batch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::testing::Time;
    use crate::CompletionHandle;

    struct Event(Time);

    impl contime_checkpoints::Event for Event {
        type Time = Time;

        fn time(&self) -> Time {
            self.0
        }
    }
    impl Input for Event {
        fn event_id(&self) -> u128 {
            self.0 .0 as u128
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(1);
        }
    }

    fn batch(time: Time, sender: &Sender<RejectionMessage<RejectionReason>>) -> RouterBatch<Event> {
        RouterBatch { inputs: crate::input::prepare_inputs(vec![Event(time)]), completion: CompletionHandle { sender: sender.clone() } }
    }

    #[test]
    fn requested_horizon_rejects_external_but_preserves_causal_work_until_proven_safe() {
        let mut frontier = Frontier::new(1);
        frontier.request(Time(100));
        let (errors, rejected) = crossbeam_channel::unbounded();
        assert!(admit::<_, ()>(batch(Time(80), &errors), None, &mut frontier).is_none());
        assert_eq!(rejected.recv().unwrap().reason, RejectionReason::BeforeHistoryHorizon);
        assert!(admit::<_, ()>(batch(Time(80), &errors), Some(Time(80)), &mut frontier).is_some());
        assert!(admit::<_, ()>(batch(Time(79), &errors), Some(Time(80)), &mut frontier).is_none());
        assert_eq!(rejected.recv().unwrap().reason, RejectionReason::BeforeSourceTime);
        let round = frontier.begin().unwrap();
        assert_eq!(frontier.report(round, 0, None), Some(Time(100)));
        assert!(admit::<_, ()>(batch(Time(80), &errors), Some(Time(80)), &mut frontier).is_none());
        assert_eq!(rejected.recv().unwrap().reason, RejectionReason::BeforeSafeTime);
        assert!(admit::<_, ()>(batch(Time(100), &errors), Some(Time(80)), &mut frontier).is_some());
    }

    #[test]
    fn configured_interval_controls_followup_rounds_without_delaying_messages() {
        for interval in [Duration::ZERO, Duration::from_secs(3600)] {
            let (input, incoming) = crossbeam_channel::unbounded();
            let (output, routed) = crossbeam_channel::unbounded();
            let (control, controls) = crossbeam_channel::unbounded();
            let (_registration, registrations) = crossbeam_channel::unbounded();
            let handle = std::thread::spawn(move || {
                run::<Event, ()>(Arc::new(incoming), output, vec![control], registrations, Time(0), 1, interval)
            });
            let (completion, _) = crossbeam_channel::unbounded();
            input.send(RouterMessage::Advance(Advance { time: Time(100), completion })).unwrap();
            assert!(matches!(routed.recv_timeout(Duration::from_secs(2)).unwrap(), RouterMessage::Advance(_)));
            let RouterMessage::Fence { round, observed } = routed.recv_timeout(Duration::from_secs(2)).unwrap() else {
                panic!("missing first round");
            };
            observed.send(round).unwrap();
            assert_eq!(controls.recv_timeout(Duration::from_secs(2)).unwrap().round, round);
            input.send(RouterMessage::Report { round, worker: 0, minimum: Some(Time(80)) }).unwrap();
            let RouterMessage::Resolve { prune: advance, .. } = routed.recv_timeout(Duration::from_secs(2)).unwrap() else {
                panic!("missing partial prune");
            };
            assert_eq!(advance.time, Time(80));
            drop(advance);

            // Query delivery and resolution completion are independent channels.
            // Zero starts another round without a timer; either may be observed first.
            let (response, _) = crossbeam_channel::unbounded();
            input.send(RouterMessage::SnapshotQuery(crate::SnapshotQuery { time: Time(100), snapshot_ids: vec![1], response })).unwrap();
            if interval.is_zero() {
                let first = routed.recv_timeout(Duration::from_secs(2)).unwrap();
                let (next, observed) = match first {
                    RouterMessage::Fence { round, observed } => {
                        assert!(matches!(routed.recv_timeout(Duration::from_secs(2)).unwrap(), RouterMessage::SnapshotQuery(_)));
                        (round, observed)
                    }
                    RouterMessage::SnapshotQuery(_) => {
                        let RouterMessage::Fence { round, observed } = routed.recv_timeout(Duration::from_secs(2)).unwrap() else {
                            panic!("zero interval must start another round");
                        };
                        (round, observed)
                    }
                    _ => panic!("expected query or next round"),
                };
                assert!(next > round);
                observed.send(next).unwrap();
                controls.recv_timeout(Duration::from_secs(2)).unwrap();
                input.send(RouterMessage::Report { round: next, worker: 0, minimum: None }).unwrap();
                let RouterMessage::Resolve { prune: advance, .. } = routed.recv_timeout(Duration::from_secs(2)).unwrap() else {
                    panic!("missing completed prune");
                };
                assert_eq!(advance.time, Time(100));
            } else {
                assert!(matches!(routed.recv_timeout(Duration::from_secs(2)).unwrap(), RouterMessage::SnapshotQuery(_)));
            }
            input.send(RouterMessage::Shutdown).unwrap();
            handle.join().unwrap();
        }
    }

    #[test]
    fn coordinator_requires_marker_and_all_worker_reports_before_pruning() {
        let (input, incoming) = crossbeam_channel::unbounded();
        let (output, routed) = crossbeam_channel::unbounded();
        let (control, controls) = crossbeam_channel::unbounded();
        let (_registration, registrations) = crossbeam_channel::unbounded();
        let handle = std::thread::spawn(move || {
            run::<Event, ()>(Arc::new(incoming), output, vec![control], registrations, Time(0), 2, Duration::from_millis(100))
        });
        let (completion, _) = crossbeam_channel::unbounded();
        input.send(RouterMessage::Advance(Advance { time: Time(100), completion })).unwrap();
        assert!(matches!(routed.recv_timeout(Duration::from_secs(1)).unwrap(), RouterMessage::Advance(_)));
        let RouterMessage::Fence { round, observed } = routed.recv_timeout(Duration::from_secs(1)).unwrap() else {
            panic!("missing marker");
        };
        assert!(controls.try_recv().is_err());
        observed.send(round).unwrap();
        assert_eq!(controls.recv_timeout(Duration::from_secs(1)).unwrap().round, round);
        input.send(RouterMessage::Report { round, worker: 0, minimum: None }).unwrap();
        // A query is a FIFO probe: it must pass through without waiting for the
        // missing worker, and no prune may have preceded it.
        let (response, _) = crossbeam_channel::unbounded();
        input.send(RouterMessage::SnapshotQuery(crate::SnapshotQuery { time: Time(100), snapshot_ids: vec![1], response })).unwrap();
        assert!(matches!(routed.recv_timeout(Duration::from_secs(1)).unwrap(), RouterMessage::SnapshotQuery(_)));
        let (errors, _) = crossbeam_channel::unbounded();
        input.send(RouterMessage::Internal { source: Time(80), batch: batch(Time(80), &errors) }).unwrap();
        input.send(RouterMessage::Report { round, worker: 1, minimum: None }).unwrap();
        assert!(matches!(routed.recv_timeout(Duration::from_secs(1)).unwrap(), RouterMessage::Apply(_)));
        let RouterMessage::Resolve { prune: advance, .. } = routed.recv_timeout(Duration::from_secs(1)).unwrap() else {
            panic!("missing prune");
        };
        assert_eq!(advance.time, Time(0));
        input.send(RouterMessage::Shutdown).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn completed_horizon_requires_every_worker_and_ignores_older_reports() {
        let (input, incoming) = crossbeam_channel::unbounded();
        let (output, _routed) = crossbeam_channel::unbounded();
        let (_registration, registrations) = crossbeam_channel::unbounded();
        let handle = std::thread::spawn(move || {
            run::<Event, ()>(Arc::new(incoming), output, vec![], registrations, Time(0), 2, Duration::from_millis(100))
        });
        let (updates, horizons) = crossbeam_channel::unbounded();
        input.send(RouterMessage::SubscribePrunedHorizon(updates)).unwrap();
        assert_eq!(horizons.recv_timeout(Duration::from_secs(2)).unwrap(), Time(0));
        // Registration is a FIFO barrier for the preceding completion reports.
        let current = || {
            let (sender, receiver) = crossbeam_channel::unbounded();
            input.send(RouterMessage::SubscribePrunedHorizon(sender)).unwrap();
            receiver.recv_timeout(Duration::from_secs(2)).unwrap()
        };
        input.send(RouterMessage::Pruned { worker: 0, horizon: Time(100) }).unwrap();
        assert_eq!(current(), Time(0));
        assert!(horizons.is_empty());
        input.send(RouterMessage::Pruned { worker: 1, horizon: Time(80) }).unwrap();
        assert_eq!(horizons.recv_timeout(Duration::from_secs(2)).unwrap(), Time(80));
        input.send(RouterMessage::Pruned { worker: 1, horizon: Time(50) }).unwrap();
        input.send(RouterMessage::Pruned { worker: 0, horizon: Time(100) }).unwrap();
        assert_eq!(current(), Time(80));
        assert!(horizons.is_empty());
        input.send(RouterMessage::Pruned { worker: 1, horizon: Time(120) }).unwrap();
        assert_eq!(horizons.recv_timeout(Duration::from_secs(2)).unwrap(), Time(100));
        input.send(RouterMessage::Shutdown).unwrap();
        handle.join().unwrap();
        assert_eq!(horizons.recv(), Err(crossbeam_channel::RecvError));
    }
}
