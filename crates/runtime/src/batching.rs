//! ConTime protocol adapters for the isolated transport batcher.
use crate::{CompletionHandle, RouterBatch, RouterMessage, SharedEvent};
use crossbeam_channel::{Receiver, Sender};

#[cfg(test)]
use contime_batcher::MAX_REQUESTS;
pub(crate) use contime_batcher::{GATHER_WORK, MAX_EVENTS};

impl<I: crate::ThreadInput> contime_batcher::Batch for RouterBatch<I> {
    type Item = SharedEvent<I>;
    type Context = CompletionHandle;
    fn into_parts(self) -> (Vec<Self::Item>, Self::Context) {
        (self.inputs, self.completion)
    }
    fn from_parts(inputs: Vec<Self::Item>, completion: Self::Context) -> Self {
        Self { inputs, completion }
    }
    fn items(&mut self) -> &mut Vec<Self::Item> {
        &mut self.inputs
    }
    fn can_merge(&self, other: &Self) -> bool {
        self.completion.sender.same_channel(&other.completion.sender)
    }
}
impl<I: crate::ThreadInput, S> contime_batcher::Request for RouterMessage<I, S> {
    type Batch = RouterBatch<I>;
    fn into_batch(self) -> Result<Self::Batch, Self> {
        match self {
            Self::Apply(batch) => Ok(batch),
            other => Err(other),
        }
    }
}
fn emit<I: crate::ThreadInput, S>(output: &Sender<RouterMessage<I, S>>, mut segments: Vec<RouterBatch<I>>) -> Result<(), ()> {
    let message = match segments.len() {
        1 => RouterMessage::Apply(segments.pop().unwrap()),
        _ => RouterMessage::ApplyPacket(segments),
    };
    output.send(message).map_err(|_| ())
}
type Packet<I> = Vec<RouterBatch<I>>;

pub(crate) fn batcher<I: crate::ThreadInput, S>(
    output: &Sender<RouterMessage<I, S>>,
    limit: usize,
) -> contime_batcher::Batcher<RouterBatch<I>, impl FnMut(Packet<I>) -> Result<(), ()> + '_> {
    contime_batcher::Batcher::new(move |segments| emit(output, segments), limit)
}
pub(crate) fn drain<I: crate::ThreadInput, S>(
    first: RouterBatch<I>,
    input: &Receiver<RouterMessage<I, S>>,
    output: &Sender<RouterMessage<I, S>>,
    admit: impl FnMut(RouterBatch<I>) -> Option<RouterBatch<I>>,
    may_gather: impl FnMut() -> bool,
) -> Result<Option<RouterMessage<I, S>>, ()> {
    contime_batcher::drain(first, || input.try_recv().ok(), |segments| emit(output, segments), admit, may_gather)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::testing::Time;
    use crate::{Input, RouterBatch, RouterMessage};
    use contime_api::ApplyOutput;
    use crossbeam_channel::{unbounded, TryRecvError};

    struct Event(u128);
    impl crate::checkpoints::Event for Event {
        type Time = Time;
        fn time(&self) -> Time {
            Time(1)
        }
    }
    impl Input for Event {
        fn event_id(&self) -> u128 {
            self.0
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(self.0);
        }
    }
    fn packet(message: RouterMessage<Event, ()>) -> Vec<RouterBatch<Event>> {
        match message {
            RouterMessage::Apply(batch) => vec![batch],
            RouterMessage::ApplyPacket(batches) => batches,
            _ => panic!("expected apply packet"),
        }
    }
    #[test]
    fn internal_boundaries_ignore_original_submission_sizes() {
        let (output, received) = unbounded();
        let mut batcher = batcher(&output, 4);
        let (a, ar) = unbounded();
        let (b, br) = unbounded();
        let (c, cr) = unbounded();
        batcher.push(RouterBatch::create((0..5).map(|i| Event(i).into()).collect(), a)).unwrap();
        batcher.push(RouterBatch::create(vec![Event(5).into()], b)).unwrap();
        batcher.push(RouterBatch::create(vec![Event(6).into(), Event(7).into()], c)).unwrap();
        batcher.flush().unwrap();
        let mut packets = received.try_iter().map(packet).collect::<Vec<_>>();
        assert_eq!(packets.len(), 2);
        assert_eq!(packets.iter().map(|p| p.iter().map(|b| b.inputs.len()).sum::<usize>()).collect::<Vec<_>>(), [4, 4]);
        assert_eq!(
            packets.iter().flatten().flat_map(|b| b.inputs.iter().map(|e| e.event_id())).collect::<Vec<_>>(),
            (0..8).collect::<Vec<_>>()
        );
        drop(packets.remove(0));
        assert_eq!(ar.try_recv(), Err(TryRecvError::Empty));
        drop(packets);
        assert_eq!(ar.try_recv(), Err(TryRecvError::Disconnected));
        assert_eq!(br.try_recv(), Err(TryRecvError::Disconnected));
        assert_eq!(cr.try_recv(), Err(TryRecvError::Disconnected));
    }
    #[test]
    fn empty_request_releases_its_response_without_waiting_for_another_arrival() {
        let (output, received) = unbounded::<RouterMessage<Event, ()>>();
        let mut batcher = batcher(&output, 4);
        let (response, completed) = unbounded();
        batcher.push(RouterBatch::create(vec![], response)).unwrap();
        batcher.flush().unwrap();
        assert!(received.is_empty());
        assert_eq!(completed.try_recv(), Err(TryRecvError::Disconnected));
    }
    #[test]
    fn shared_response_segments_can_merge_without_per_event_receipts() {
        let (output, received) = unbounded::<RouterMessage<Event, ()>>();
        let mut batcher = batcher(&output, 4);
        let (response, completed) = unbounded();
        for id in 0..4 {
            batcher.push(RouterBatch::create(vec![Event(id).into()], response.clone())).unwrap();
        }
        drop(response);
        batcher.flush().unwrap();
        let segments = packet(received.try_recv().unwrap());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].inputs.len(), 4);
        drop(segments);
        assert_eq!(completed.try_recv(), Err(TryRecvError::Disconnected));
    }
    #[test]
    fn draining_stops_at_controls_and_flushes_on_empty_or_disconnect() {
        for disconnect in [false, true] {
            let (input, incoming) = unbounded();
            let (output, received) = unbounded::<RouterMessage<Event, ()>>();
            let (response, _responses) = unbounded();
            input.send(RouterMessage::Apply(RouterBatch::create(vec![Event(2).into()], response.clone()))).ok().unwrap();
            input.send(RouterMessage::Shutdown).ok().unwrap();
            input.send(RouterMessage::Apply(RouterBatch::create(vec![Event(3).into()], response.clone()))).ok().unwrap();
            let boundary =
                drain(RouterBatch::create(vec![Event(1).into()], response.clone()), &incoming, &output, Some, || true).ok().unwrap();
            assert!(matches!(boundary, Some(RouterMessage::Shutdown)));
            let segments = packet(received.try_recv().unwrap());
            assert_eq!(segments.iter().map(|b| b.inputs.len()).sum::<usize>(), 2);
            let RouterMessage::Apply(next) = incoming.try_recv().unwrap() else { panic!("following request must remain queued") };
            if disconnect {
                drop(input);
            }
            assert!(drain(next, &incoming, &output, Some, || true).ok().unwrap().is_none());
            assert_eq!(packet(received.try_recv().unwrap())[0].inputs.len(), 1);
        }
    }
    #[test]
    fn gathering_budget_does_not_consume_the_next_request() {
        let (input, incoming) = unbounded();
        let (output, received) = unbounded::<RouterMessage<Event, ()>>();
        let (response, _completed) = unbounded();
        input.send(RouterMessage::Apply(RouterBatch::create(vec![Event(2).into()], response.clone()))).ok().unwrap();
        drain(RouterBatch::create(vec![Event(1).into()], response), &incoming, &output, Some, || false).ok().unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(packet(received.try_recv().unwrap())[0].inputs[0].event_id(), 1);
    }
    #[test]
    fn hot_queue_yields_after_a_bounded_number_of_requests() {
        let (input, incoming) = unbounded();
        let (output, received) = unbounded::<RouterMessage<Event, ()>>();
        let (response, _responses) = unbounded();
        for id in 1..=MAX_REQUESTS + 10 {
            input.send(RouterMessage::Apply(RouterBatch::create(vec![Event(id as u128).into()], response.clone()))).ok().unwrap();
        }
        drain(RouterBatch::create(vec![Event(0).into()], response), &incoming, &output, Some, || true).ok().unwrap();
        assert_eq!(incoming.len(), 11);
        assert_eq!(received.try_iter().flat_map(packet).map(|b| b.inputs.len()).sum::<usize>(), MAX_REQUESTS);
    }
}
