use crossbeam_channel::Sender;

use crate::{ApiError, ConTime, Input, RouterMessage, TrackedEvent};

impl<I, S, W> ConTime<I, S, W>
where
    I: Input,
{
    pub fn send_query_at(
        &self,
        time: I::Time,
        snapshot_ids: impl IntoIterator<Item = u128>,
        response: Sender<Vec<Box<S>>>,
    ) -> Result<(), ApiError> {
        contime_api::send_query_at::<RouterMessage<I, S>, _, _, _>(self.runtime.input(), time, snapshot_ids, response)
    }

    pub fn query_at(&self, time: I::Time, snapshot_ids: impl IntoIterator<Item = u128>) -> Result<Vec<Box<S>>, ApiError> {
        contime_api::query_at::<RouterMessage<I, S>, _, _, _>(self.runtime.input(), time, snapshot_ids)
    }

    pub fn send_query_events_between(
        &self,
        snapshot_id: u128,
        from: I::Time,
        to: I::Time,
        response: Sender<Vec<TrackedEvent<I>>>,
    ) -> Result<(), ApiError> {
        contime_api::send_query_events_between::<RouterMessage<I, S>, _, _>(self.runtime.input(), snapshot_id, from, to, response)
    }

    pub fn query_events_between(&self, snapshot_id: u128, from: I::Time, to: I::Time) -> Result<Vec<TrackedEvent<I>>, ApiError> {
        contime_api::query_events_between::<RouterMessage<I, S>, _, _>(self.runtime.input(), snapshot_id, from, to)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use contime_checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot};
    use contime_memory::ConservativeTrackedSize;
    use crossbeam_channel::unbounded;

    use crate::{ConTime, ConTimeConfig, Input};

    struct TestEvent {
        id: u128,
        time: u64,
        value: u64,
    }

    impl ConservativeTrackedSize for TestEvent {
        fn conservative_tracked_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    impl Input for TestEvent {
        type Time = u64;

        fn event_id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> Self::Time {
            self.time
        }

        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            emit(7);
        }
    }

    #[derive(Clone, Default)]
    struct TestSnapshot {
        snapshot_id: u128,
        time: u64,
        value: u64,
    }

    impl ConservativeTrackedSize for TestSnapshot {
        fn conservative_tracked_size(&self) -> usize {
            std::mem::size_of::<Self>()
        }
    }

    impl Snapshot for TestSnapshot {
        type Time = u64;

        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }
    }

    impl ApplyEvents<TestEvent> for TestSnapshot {
        fn create(snapshot_id: u128, _first_event: &TestEvent) -> Self {
            Self { snapshot_id, ..Self::default() }
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, TestEvent>) {
            self.value += batch.events.iter().map(|event| event.value).sum::<u64>();
        }
    }

    fn config() -> ConTimeConfig {
        ConTimeConfig {
            router_count: 2,
            worker_count: 4,
            router_seed: 9,
            memory_limit: 1_000_000,
            memory_buffer: 1_000,
            worker: contime_worker::WorkerConfig {
                maximum_dirty_age: Duration::from_micros(100),
                replays_per_receive: 1,
                deadline_compaction_minimum: 1_024,
                deadline_compaction_multiplier: 2,
            },
            checkpoints: CheckpointConfig { interval: 2 },
        }
    }

    #[test]
    fn snapshot_and_event_queries_share_the_complete_runtime_pipeline() {
        let contime = ConTime::<TestEvent, TestSnapshot, ()>::start(config(), ()).unwrap();
        contime
            .apply([
                TestEvent { id: 1, time: 10, value: 1 },
                TestEvent { id: 2, time: 20, value: 2 },
                TestEvent { id: 3, time: 30, value: 4 },
            ])
            .unwrap();

        let snapshots = contime.query_at(20, [7, 999]).unwrap();
        let events = contime.query_events_between(7, 10, 30).unwrap();

        assert_eq!(snapshots.len(), 1);
        assert_eq!((snapshots[0].snapshot_id, snapshots[0].time, snapshots[0].value), (7, 20, 3));
        assert_eq!(events.iter().map(|event| event.event_id()).collect::<Vec<_>>(), vec![1, 2]);

        let (snapshot_response, async_snapshots) = unbounded();
        contime.send_query_at(30, [7], snapshot_response).unwrap();
        assert_eq!(async_snapshots.recv().unwrap()[0].value, 7);

        let (event_response, async_events) = unbounded();
        contime.send_query_events_between(7, 20, 31, event_response).unwrap();
        assert_eq!(async_events.recv().unwrap().iter().map(|event| event.event_id()).collect::<Vec<_>>(), vec![2, 3]);
        contime.shutdown();
    }
}
