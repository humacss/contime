//! Read retained snapshots and event ranges without publishing effects.

use crossbeam_channel::Sender;

use crate::{ApiError, ConTime, Input, RouterMessage, SharedEvent};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    /// Enqueues a read of the requested snapshots through `time`, inclusively.
    ///
    /// Results arrive in worker batches; their order is unspecified and missing
    /// snapshot IDs are omitted. The request is finished when all response senders
    /// close. Drop any caller-held sender clone to observe that closure. Drain a
    /// bounded response channel concurrently to avoid blocking the worker.
    /// This reconstructs available history without publishing effects or waiting
    /// for other work to settle. Returns [`ApiError`] if submission fails.
    pub fn send_query_at(
        &self,
        time: I::Time,
        snapshot_ids: impl IntoIterator<Item = u128>,
        response: Sender<Vec<Box<S>>>,
    ) -> Result<(), ApiError> {
        contime_api::send_query_at::<RouterMessage<I, S>, _, _, _>(&self.input, time, snapshot_ids, response)
    }

    /// Collects a snapshot query through `time`, including events exactly at it.
    ///
    /// Waits for this query's responses, not for global replay completion. Missing
    /// IDs and times before retained history produce no snapshot; result order is
    /// unspecified. Observe the insertion responses for submitted inputs before
    /// querying their state. For settled-state assertions in tests or benchmarks,
    /// stop producers and use [`Self::wait_until_idle`]; running applications
    /// should observe operation-specific responses or progress instead.
    /// Returns [`ApiError`] if submission fails.
    pub fn query_at(&self, time: I::Time, snapshot_ids: impl IntoIterator<Item = u128>) -> Result<Vec<Box<S>>, ApiError> {
        contime_api::query_at::<RouterMessage<I, S>, _, _, _>(&self.input, time, snapshot_ids)
    }

    /// Enqueues a retained event-history read in the half-open range `[from, to)`.
    ///
    /// Shared event handles arrive in worker batches. A range with `from >= to`
    /// is empty. Events already pruned cannot be returned. Drain bounded channels
    /// concurrently and drop caller-held sender clones to observe completion by
    /// closure. No processing target is advanced. Returns [`ApiError`] on enqueue
    /// failure; successful enqueue alone does not mean insertion or replay settled.
    pub fn send_query_events_between(
        &self,
        snapshot_id: u128,
        from: I::Time,
        to: I::Time,
        response: Sender<Vec<SharedEvent<I>>>,
    ) -> Result<(), ApiError> {
        contime_api::send_query_events_between::<RouterMessage<I, S>, _, _>(&self.input, snapshot_id, from, to, response)
    }

    /// Collects retained events for one snapshot in the half-open range `[from, to)`.
    ///
    /// Events are ordered by timestamp then stable event ID within the history.
    /// Missing histories and ranges with `from >= to` return an empty vector.
    /// Returned handles share immutable input ownership; pruned events are absent.
    /// Waits only for this read and returns [`ApiError`] if submission fails.
    pub fn query_events_between(&self, snapshot_id: u128, from: I::Time, to: I::Time) -> Result<Vec<SharedEvent<I>>, ApiError> {
        contime_api::query_events_between::<RouterMessage<I, S>, _, _>(&self.input, snapshot_id, from, to)
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::Time;
    use std::time::Duration;

    use crate::checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};

    use crossbeam_channel::unbounded;

    use crate::{ConTime, ConTimeConfig, Input};

    struct TestEvent {
        id: u128,
        time: Time,
        value: u64,
    }

    impl crate::checkpoints::Event for TestEvent {
        type Time = Time;

        fn time(&self) -> Self::Time {
            self.time
        }
    }
    impl Input for TestEvent {
        fn event_id(&self) -> u128 {
            self.id
        }
        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }

    #[derive(Clone, Default)]
    struct TestSnapshot {
        snapshot_id: u128,
        time: Time,
        value: u64,
    }

    impl Snapshot for TestSnapshot {
        type Time = Time;

        fn time(&self) -> &Self::Time {
            &self.time
        }
        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    impl ApplyEvents<TestEvent> for TestSnapshot {
        fn create(snapshot_id: u128, _first_event: &TestEvent) -> Self {
            Self { snapshot_id, ..Self::default() }
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Self::Time, TestEvent>) {
            self.value += batch.events.map(|event| event.value).sum::<u64>();
        }
    }

    fn config() -> ConTimeConfig<Time> {
        ConTimeConfig {
            router_count: 2,
            worker_count: 4,
            placement: contime_router::Placement::default(),
            pruning_interval: std::time::Duration::from_millis(100),

            history_retention: Time(0),
            checkpoints: CheckpointConfig { interval: 2 },
        }
    }

    #[test]
    fn snapshot_and_event_queries_share_the_complete_runtime_pipeline() {
        let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(), ()).unwrap();
        contime
            .apply([
                TestEvent { id: 1, time: Time(10), value: 1 },
                TestEvent { id: 2, time: Time(20), value: 2 },
                TestEvent { id: 3, time: Time(30), value: 4 },
            ])
            .unwrap();

        contime.wait_until_idle(Duration::from_secs(2)).unwrap();
        let snapshots = contime.query_at(Time(20), [7, 999]).unwrap();
        let events = contime.query_events_between(7, Time(10), Time(30)).unwrap();

        assert_eq!(snapshots.len(), 1);
        assert_eq!((snapshots[0].snapshot_id, snapshots[0].time, snapshots[0].value), (7, Time(20), 3));
        assert_eq!(events.iter().map(|event| event.event_id()).collect::<Vec<_>>(), vec![1, 2]);

        let (snapshot_response, async_snapshots) = unbounded();
        contime.send_query_at(Time(30), [7], snapshot_response).unwrap();
        assert_eq!(async_snapshots.recv().unwrap()[0].value, 7);

        let (event_response, async_events) = unbounded();
        contime.send_query_events_between(7, Time(20), Time(31), event_response).unwrap();
        assert_eq!(async_events.recv().unwrap().iter().map(|event| event.event_id()).collect::<Vec<_>>(), vec![2, 3]);
        contime.shutdown();
    }
}
