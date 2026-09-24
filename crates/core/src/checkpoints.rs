//! Iterator-based application hooks bridging Core consumers to snapshot playback.
pub use contime_checkpoints::*;

/// Startup checkpoint spacing used when core creates each snapshot's store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointConfig {
    /// Number of events between checkpoints; zero leaves playback intervals unbounded.
    pub interval: u64,
}

/// One complete same-time event bucket.
pub struct EventBatch<'a, 'e, T, E> {
    pub snapshot_id: u128,
    pub time: T,
    pub events: &'a mut dyn Iterator<Item = &'e E>,
}

/// One effective event batch selected from a canonical timestamp bucket.
pub struct ApplyBatch<'a, 'e, T, E> {
    pub snapshot_id: u128,
    pub time: T,
    /// Cumulative raw history event count before this canonical timestamp batch.
    pub history_event_count: u64,
    pub events: &'a mut dyn Iterator<Item = &'e E>,
}

/// Consumer-provided snapshot materialization and event application behavior.
pub trait ApplyEvents<E>: Snapshot {
    /// Creates clean state with the identity selected by the first event.
    fn create(snapshot_id: u128, first_event: &E) -> Self;

    /// Applies one effective event batch.
    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Self::Time, E>);
}

/// The only mutable snapshot access exposed to apply wrappers.
pub struct ApplyInner<'a, S>
where
    S: Snapshot,
{
    pub(crate) snapshot: &'a mut S,
    pub(crate) history_event_count: u64,
    pub(crate) apply_count: usize,
}

/// Infallible extension seam around same-timestamp snapshot application.
///
/// Implementations must call `ApplyInner::apply_event_batch` at least once.
/// They may filter or partition the canonical batch and may use an empty
/// effective batch to suppress every event.
pub trait ApplyWrapper<S, E>
where
    S: ApplyEvents<E>,
{
    fn apply_event_batch(&mut self, batch: EventBatch<'_, '_, S::Time, E>, apply_inner: &mut ApplyInner<'_, S>) {
        apply_inner.apply_event_batch(batch);
    }

    /// Applies a batch while replaying changes to retained event history.
    /// Override to publish effects after application. Pruning also calls this
    /// hook as it moves the anchor; queries call only `apply_event_batch`.
    fn replay_event_batch(&mut self, batch: EventBatch<'_, '_, S::Time, E>, apply_inner: &mut ApplyInner<'_, S>) {
        self.apply_event_batch(batch, apply_inner);
    }

    /// Compact consumer-owned data when the retained history boundary advances.
    /// Called for the moving anchor after each pruned timestamp bucket and once
    /// at the final horizon predecessor, including quiet intervals. Later checkpoints are
    /// untouched. Same-time events form one atomic batch. This never runs
    /// during ordinary application, replay, or query reconstruction.
    /// Preserve snapshot time and all state needed to query or replay at/after
    /// `horizon`. Mutations must not affect clones held by other readers.
    /// Its default is a no-op. The caller owns scheduling of any resulting work.
    fn retain_snapshot(&mut self, _snapshot: &mut S, _horizon: &S::Time) {}
}

impl<S, E> ApplyWrapper<S, E> for () where S: ApplyEvents<E> {}

impl<'a, S> ApplyInner<'a, S>
where
    S: Snapshot,
{
    pub(crate) fn new(snapshot: &'a mut S, history_event_count: u64) -> Self {
        Self { snapshot, history_event_count, apply_count: 0 }
    }

    /// Returns the raw event count before the current canonical timestamp batch.
    pub const fn history_event_count(&self) -> u64 {
        self.history_event_count
    }

    /// Applies one effective batch selected by the wrapper.
    ///
    /// Every effective partition receives the same preceding raw history
    /// count. Snapshot playback updates the canonical count after the wrapper
    /// returns. An empty effective batch still advances the snapshot timestamp.
    pub fn apply_event_batch<E>(&mut self, batch: EventBatch<'_, '_, S::Time, E>) -> u64
    where
        S: ApplyEvents<E>,
    {
        self.apply_event_batch_with(batch, S::apply_events)
    }

    /// Applies a batch with a caller-supplied function, which may borrow external
    /// execution context. The function must preserve deterministic snapshot
    /// results across live and reconstruction replay. Empty batches do not invoke
    /// it. Timestamp assignment and participation accounting remain owned here.
    pub fn apply_event_batch_with<E>(
        &mut self,
        batch: EventBatch<'_, '_, S::Time, E>,
        apply: impl FnOnce(&mut S, ApplyBatch<'_, '_, S::Time, E>),
    ) -> u64 {
        let time = batch.time.clone();
        let mut events = batch.events.peekable();
        if events.peek().is_some() {
            apply(
                self.snapshot,
                ApplyBatch {
                    snapshot_id: batch.snapshot_id,
                    time: batch.time,
                    history_event_count: self.history_event_count,
                    events: &mut events,
                },
            );
        }
        self.snapshot.set_time(time);

        self.apply_count += 1;
        self.history_event_count
    }

    /// Returns the snapshot after all effective applications completed so far.
    pub fn snapshot(&self) -> &S {
        self.snapshot
    }

    pub(crate) const fn has_applied(&self) -> bool {
        self.apply_count != 0
    }
}

/// Applies one canonical same-timestamp bucket through an injected wrapper.
fn apply_batch<S, E, W>(snapshot: &mut S, batch: EventBatch<'_, '_, S::Time, E>, history_event_count: u64, wrapper: &mut W)
where
    S: ApplyEvents<E>,
    W: ApplyWrapper<S, E>,
{
    let mut apply_inner = ApplyInner::new(snapshot, history_event_count);
    wrapper.apply_event_batch(batch, &mut apply_inner);
    assert!(apply_inner.has_applied(), "an apply wrapper must call the inner apply at least once per event batch");
}

/// Borrowed integration context; effect selection is not a checkpoint policy.
pub(crate) struct Application<'a, W> {
    pub snapshot_id: u128,
    pub wrapper: std::cell::RefCell<&'a mut W>,
    pub live: bool,
}

impl<I: crate::Input> contime_checkpoints::Event for crate::SharedEvent<I> {
    type Time = I::Time;
    fn time(&self) -> Self::Time {
        self.inner.time()
    }
}
impl<I, S, W> Apply<Checkpoint<S>, Application<'_, W>> for crate::SharedEvent<I>
where
    I: crate::Input,
    S: ApplyEvents<I> + Snapshot<Time = I::Time>,
    W: ApplyWrapper<S, I>,
{
    fn apply<'a>(state: &mut Checkpoint<S>, events: impl Iterator<Item = &'a Self>, context: &Application<'_, W>)
    where
        Self: 'a,
    {
        let mut events = events.peekable();
        let Some(time) = events.peek().map(|event| event.inner.time()) else { return };
        let mut payloads = events.map(|event| event.inner.as_ref());
        let count = state.history_event_count;
        let batch = EventBatch { snapshot_id: context.snapshot_id, time, events: &mut payloads };
        if context.live {
            let mut inner = ApplyInner::new(&mut state.snapshot, count);
            context.wrapper.borrow_mut().replay_event_batch(batch, &mut inner);
            assert!(inner.apply_count > 0, "application wrapper must apply at least one batch");
        } else {
            apply_batch(&mut state.snapshot, batch, count, &mut **context.wrapper.borrow_mut());
        }
    }
}
#[cfg(test)]
mod wrapper_tests {
    use super::*;
    use crate::types::testing::Time;

    struct TestEvent(i64);
    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: Time,
        sum: i64,
        history_counts: Vec<u64>,
    }
    impl Snapshot for TestSnapshot {
        type Time = Time;
        fn time(&self) -> &Time {
            &self.time
        }
        fn set_time(&mut self, time: Time) {
            self.time = time;
        }
    }
    impl ApplyEvents<TestEvent> for TestSnapshot {
        fn create(_: u128, _: &TestEvent) -> Self {
            Self::default()
        }
        fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Time, TestEvent>) {
            self.sum += batch.events.map(|event| event.0).sum::<i64>();
            self.history_counts.push(batch.history_event_count);
        }
    }
    struct FilterEven;
    impl ApplyWrapper<TestSnapshot, TestEvent> for FilterEven {
        fn apply_event_batch(&mut self, batch: EventBatch<'_, '_, Time, TestEvent>, inner: &mut ApplyInner<'_, TestSnapshot>) {
            let mut events = batch.events.filter(|event| event.0 % 2 == 0);
            inner.apply_event_batch(EventBatch { snapshot_id: batch.snapshot_id, time: batch.time, events: &mut events });
        }
    }
    struct SkipInner;
    impl ApplyWrapper<TestSnapshot, TestEvent> for SkipInner {
        fn apply_event_batch(&mut self, _: EventBatch<'_, '_, Time, TestEvent>, _: &mut ApplyInner<'_, TestSnapshot>) {}
    }

    #[test]
    fn happy() {
        let expected_sum = 6;
        let expected_time = Time(10);
        let expected_counts = vec![7];

        let events = [TestEvent(1), TestEvent(2), TestEvent(3)];
        let mut snapshot = TestSnapshot::default();

        apply_batch(&mut snapshot, EventBatch { snapshot_id: 1, time: expected_time, events: &mut events.iter() }, 7, &mut ());

        assert_eq!(snapshot.sum, expected_sum);
        assert_eq!(snapshot.time, expected_time);
        assert_eq!(snapshot.history_counts, expected_counts);
    }

    #[test]
    fn filtering_preserves_the_preceding_canonical_count() {
        let expected_sum = 6;
        let expected_counts = vec![7];

        let events = [TestEvent(1), TestEvent(2), TestEvent(3), TestEvent(4)];
        let mut snapshot = TestSnapshot::default();

        apply_batch(&mut snapshot, EventBatch { snapshot_id: 1, time: Time(10), events: &mut events.iter() }, 7, &mut FilterEven);

        assert_eq!(snapshot.sum, expected_sum);
        assert_eq!(snapshot.history_counts, expected_counts);
    }

    #[test]
    fn callback_borrows_context() {
        let expected_sum = 6;
        let expected_time = Time(10);
        let expected_count = 7;

        let events = [TestEvent(2)];
        let mut snapshot = TestSnapshot::default();
        let multiplier = 3;
        let mut inner = ApplyInner::new(&mut snapshot, expected_count);

        let actual_count = inner.apply_event_batch_with(
            EventBatch { snapshot_id: 1, time: expected_time, events: &mut events.iter() },
            |snapshot, batch| {
                snapshot.sum += batch.events.map(|event| event.0 * multiplier).sum::<i64>();
            },
        );

        assert_eq!(actual_count, expected_count);
        assert!(inner.has_applied());
        assert_eq!(inner.snapshot().sum, expected_sum);
        assert_eq!(inner.snapshot().time, expected_time);
    }

    #[test]
    fn empty_effective_batch_advances_time_without_calling_the_consumer() {
        let expected_time = Time(20);
        let expected_count = 7;

        let mut snapshot = TestSnapshot::default();
        let mut inner = ApplyInner::new(&mut snapshot, expected_count);

        inner.apply_event_batch_with::<TestEvent>(
            EventBatch { snapshot_id: 1, time: expected_time, events: &mut std::iter::empty() },
            |_, _| panic!("empty effective batch must not call the consumer"),
        );

        assert!(inner.has_applied());
        assert_eq!(inner.history_event_count(), expected_count);
        assert_eq!(inner.snapshot().time, expected_time);
        assert_eq!(inner.snapshot().sum, 0);
    }

    #[test]
    #[should_panic(expected = "an apply wrapper must call the inner apply")]
    fn wrapper_skipping_inner_application_is_rejected() {
        let events = [TestEvent(1)];
        let mut snapshot = TestSnapshot::default();

        apply_batch(&mut snapshot, EventBatch { snapshot_id: 1, time: Time(10), events: &mut events.iter() }, 0, &mut SkipInner);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_apply() {
        let events = (0..1_000).map(TestEvent).collect::<Vec<_>>();
        let mut criterion = criterion::Criterion::default();
        criterion.bench_function("core/wrapper/1000_events/one_batch", |bencher| {
            bencher.iter(|| {
                let mut snapshot = TestSnapshot::default();
                apply_batch(
                    &mut snapshot,
                    EventBatch { snapshot_id: 1, time: Time(10), events: &mut std::hint::black_box(&events).iter() },
                    0,
                    &mut (),
                );
                std::hint::black_box(snapshot)
            });
        });
        criterion.final_summary();
    }
}
