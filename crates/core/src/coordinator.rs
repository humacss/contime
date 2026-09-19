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
) {
    let mut frontier = Frontier::new(workers);
    let mut target = I::Time::default();
    let (observed, observations) = crossbeam_channel::unbounded();
    let mut listeners: Vec<Sender<bool>> = Vec::new();
    let mut working = false;
    let mut next_round = Instant::now();
    let mut awaiting_marker = None;
    loop {
        let pruning = frontier.requested() > frontier.safe();
        if pruning && !frontier.measuring() && Instant::now() >= next_round {
            if !working {
                working = true;
                listeners.retain(|listener| listener.send(true).is_ok());
            }
            if let Some(round) = frontier.begin() {
                awaiting_marker = Some(round);
                if output.send(RouterMessage::Fence { round, observed: observed.clone() }).is_err() {
                    return;
                }
                next_round = Instant::now() + Duration::from_millis(100);
            }
        }
        let busy = pruning || !input.is_empty() || !observations.is_empty();
        if working != busy {
            working = busy;
            listeners.retain(|listener| listener.send(working).is_ok());
        }
        let timer = if pruning && !frontier.measuring() {
            crossbeam_channel::after(next_round.saturating_duration_since(Instant::now()))
        } else {
            crossbeam_channel::never()
        };
        let mut ready = crossbeam_channel::Select::new();
        let message_index = ready.recv(&input);
        let observed_index = ready.recv(&observations);
        let registration_index = ready.recv(&registrations);
        ready.recv(&timer);
        // Publish working before removing a queued message, so idle observers
        // cannot mistake a handoff for an empty pipeline.
        let selected = ready.ready();
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
            RouterMessage::Apply(batch) => admit(batch, None, &mut frontier),
            RouterMessage::Internal { source, batch } => admit(batch, Some(source), &mut frontier),
            RouterMessage::Advance(mut advance) => {
                target = target.max(advance.time);
                frontier.request(target.saturating_sub(&retention));
                advance.time = target.clone();
                Some(RouterMessage::Advance(advance))
            }
            RouterMessage::Report { round, worker, minimum } => frontier.report(round, worker, minimum).map(|time| {
                let (completion, _) = crossbeam_channel::unbounded();
                RouterMessage::Prune(Advance { time, completion })
            }),
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

fn admit<I: Input, S>(mut batch: RouterBatch<I>, source: Option<I::Time>, frontier: &mut Frontier<I::Time>) -> Option<RouterMessage<I, S>> {
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
    use crate::{CompletionHandle, MemoryBudget};
    use contime_memory::ConservativeTrackedSize;

    struct Event(u64);
    impl ConservativeTrackedSize for Event {
        fn conservative_tracked_size(&self) -> usize {
            8
        }
    }
    impl Input for Event {
        type Time = u64;
        fn event_id(&self) -> u128 {
            self.0.into()
        }
        fn time(&self) -> u64 {
            self.0
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(1);
        }
    }

    fn batch(time: u64, sender: &Sender<RejectionMessage<RejectionReason>>) -> RouterBatch<Event> {
        RouterBatch {
            inputs: crate::input::prepare_inputs(&MemoryBudget::new(100_000, 0), vec![Event(time)]).unwrap(),
            completion: CompletionHandle { sender: sender.clone() },
        }
    }

    #[test]
    fn requested_horizon_rejects_external_but_preserves_causal_work_until_proven_safe() {
        let mut frontier = Frontier::new(1);
        frontier.request(100);
        let (errors, rejected) = crossbeam_channel::unbounded();
        assert!(admit::<_, ()>(batch(80, &errors), None, &mut frontier).is_none());
        assert_eq!(rejected.recv().unwrap().reason, RejectionReason::BeforeHistoryHorizon);
        assert!(admit::<_, ()>(batch(80, &errors), Some(80), &mut frontier).is_some());
        assert!(admit::<_, ()>(batch(79, &errors), Some(80), &mut frontier).is_none());
        assert_eq!(rejected.recv().unwrap().reason, RejectionReason::BeforeSourceTime);
        let round = frontier.begin().unwrap();
        assert_eq!(frontier.report(round, 0, None), Some(100));
        assert!(admit::<_, ()>(batch(80, &errors), Some(80), &mut frontier).is_none());
        assert_eq!(rejected.recv().unwrap().reason, RejectionReason::BeforeSafeTime);
        assert!(admit::<_, ()>(batch(100, &errors), Some(80), &mut frontier).is_some());
    }

    #[test]
    fn coordinator_requires_marker_and_all_worker_reports_before_pruning() {
        let (input, incoming) = crossbeam_channel::unbounded();
        let (output, routed) = crossbeam_channel::unbounded();
        let (control, controls) = crossbeam_channel::unbounded();
        let (_registration, registrations) = crossbeam_channel::unbounded();
        let handle = std::thread::spawn(move || run::<Event, ()>(Arc::new(incoming), output, vec![control], registrations, 0, 2));
        let (completion, _) = crossbeam_channel::unbounded();
        input.send(RouterMessage::Advance(Advance { time: 100, completion })).unwrap();
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
        input.send(RouterMessage::SnapshotQuery(crate::SnapshotQuery { time: 100, snapshot_ids: vec![1], response })).unwrap();
        assert!(matches!(routed.recv_timeout(Duration::from_secs(1)).unwrap(), RouterMessage::SnapshotQuery(_)));
        let (errors, _) = crossbeam_channel::unbounded();
        input.send(RouterMessage::Internal { source: 80, batch: batch(80, &errors) }).unwrap();
        input.send(RouterMessage::Report { round, worker: 1, minimum: None }).unwrap();
        assert!(matches!(routed.recv_timeout(Duration::from_secs(1)).unwrap(), RouterMessage::Apply(_)));
        let RouterMessage::Prune(advance) = routed.recv_timeout(Duration::from_secs(1)).unwrap() else {
            panic!("missing prune");
        };
        assert_eq!(advance.time, 80);
        input.send(RouterMessage::Shutdown).unwrap();
        handle.join().unwrap();
    }
}
