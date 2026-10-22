use std::collections::VecDeque;
use std::io;

use crossbeam_channel::{Receiver, TryRecvError};

use super::{ConTime, Counter, CounterChanged, Time};
use contime::{RejectionMessage, RejectionReason};

pub struct Delivery {
    through: Time,
    events: usize,
    reordered: usize,
    inserted: Receiver<RejectionMessage<RejectionReason>>,
}

/// Submit a canonical group in reverse order. One completion receiver covers
/// every route; its closure acknowledges admission, not completed live replay.
pub fn send_reversed(history: &ConTime<CounterChanged, Counter, ()>, events: Vec<CounterChanged>) -> io::Result<Delivery> {
    let through = events.last().expect("nonempty delivery group").time;
    let count = events.len();
    let (responses, inserted) = crossbeam_channel::unbounded();
    history.send(events.into_iter().rev(), responses).map_err(|error| io::Error::other(format!("submit reversed group: {error:?}")))?;
    Ok(Delivery { through, events: count, reordered: count.saturating_sub(1), inserted })
}

#[derive(Default)]
pub struct Progress {
    pub through: Time,
    pub submitted: usize,
    pub reordered: usize,
    pending: VecDeque<Delivery>,
}

impl Progress {
    pub fn push(&mut self, delivery: Delivery) {
        self.submitted += delivery.events;
        self.reordered += delivery.reordered;
        self.pending.push_back(delivery);
    }

    pub fn poll(&mut self) -> io::Result<()> {
        while let Some(delivery) = self.pending.front() {
            match delivery.inserted.try_recv() {
                Ok(rejection) => return Err(io::Error::other(format!("unexpected event rejection in verification stream: {rejection:?}"))),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.through = delivery.through;
                    self.pending.pop_front();
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_frontier_waits_for_every_route_and_every_earlier_group() {
        let (first, first_inserted) = crossbeam_channel::unbounded();
        let (second, second_inserted) = crossbeam_channel::unbounded();
        let mut progress = Progress::default();
        progress.push(Delivery { through: Time(10), events: 2, reordered: 1, inserted: first_inserted });
        progress.push(Delivery { through: Time(20), events: 3, reordered: 2, inserted: second_inserted });
        drop(second);
        progress.poll().unwrap();
        assert_eq!(progress.through, Time(0));
        let other_route = first.clone();
        drop(first);
        progress.poll().unwrap();
        assert_eq!(progress.through, Time(0));
        drop(other_route);
        progress.poll().unwrap();
        assert_eq!(progress.through, Time(20));
        assert_eq!(progress.submitted, 5);
        assert_eq!(progress.reordered, 3);
    }

    #[test]
    fn rejection_is_fatal_and_does_not_advance_the_query_frontier() {
        let (responses, inserted) = crossbeam_channel::unbounded();
        responses.send(RejectionMessage { event_id: 1, reason: RejectionReason::BeforeHistoryHorizon }).unwrap();
        drop(responses);
        let mut progress = Progress::default();
        progress.push(Delivery { through: Time(10), events: 2, reordered: 1, inserted });
        assert!(progress.poll().is_err());
        assert_eq!(progress.through, Time(0));
    }
}
