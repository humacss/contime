use crate::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, EventBatch, Snapshot};

impl<'a, S> ApplyInner<'a, S>
where
    S: Snapshot,
{
    pub(crate) fn new(snapshot: &'a mut S, history_event_count: u64) -> Self {
        Self { snapshot, history_event_count, apply_count: 0 }
    }

    /// Returns the cumulative raw event count represented by this canonical
    /// timestamp bucket.
    pub const fn history_event_count(&self) -> u64 {
        self.history_event_count
    }

    /// Applies one effective batch selected by the wrapper.
    ///
    /// Every effective partition receives the same cumulative raw history
    /// count. An empty effective batch records wrapper participation without
    /// mutating the snapshot.
    pub fn apply_event_batch<E>(&mut self, batch: EventBatch<'_, S::Time, E>) -> u64
    where
        S: ApplyEvents<E>,
    {
        if !batch.events.is_empty() {
            let time = batch.time.clone();
            self.snapshot.apply_events(ApplyBatch {
                snapshot_id: batch.snapshot_id,
                time: batch.time,
                history_event_count: self.history_event_count,
                events: batch.events,
            });
            self.snapshot.set_time(time);
        }

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
pub fn apply<S, E, W>(snapshot: &mut S, batch: EventBatch<'_, S::Time, E>, history_event_count: u64, wrapper: &mut W)
where
    S: ApplyEvents<E>,
    W: ApplyWrapper<S, E>,
{
    let mut apply_inner = ApplyInner::new(snapshot, history_event_count);
    wrapper.apply_event_batch(batch, &mut apply_inner);
    assert!(apply_inner.has_applied(), "an apply wrapper must call the inner apply at least once per event batch");
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::Criterion;

    use super::apply;
    use crate::{ApplyBatch, ApplyEvents, ApplyInner, ApplyWrapper, EventBatch, Snapshot};

    #[derive(Clone)]
    struct TestEvent(i64);

    #[derive(Clone, Default)]
    struct TestSnapshot {
        time: i64,
        sum: i64,
        batch_sizes: Vec<usize>,
        history_counts: Vec<u64>,
    }

    impl Snapshot for TestSnapshot {
        type Time = i64;

        fn set_time(&mut self, time: Self::Time) {
            self.time = time;
        }

        fn conservative_size(&self) -> u64 {
            64
        }
    }

    impl ApplyEvents<TestEvent> for TestSnapshot {
        fn create(_snapshot_id: u128, _first_event: &TestEvent) -> Self {
            Self::default()
        }

        fn apply_events(&mut self, batch: ApplyBatch<'_, Self::Time, TestEvent>) {
            self.sum += batch.events.iter().map(|event| event.0).sum::<i64>();
            self.batch_sizes.push(batch.events.len());
            self.history_counts.push(batch.history_event_count);
        }
    }

    struct FilterEven;

    impl ApplyWrapper<TestSnapshot, TestEvent> for FilterEven {
        fn apply_event_batch(&mut self, batch: EventBatch<'_, i64, TestEvent>, apply_inner: &mut ApplyInner<'_, TestSnapshot>) {
            let filtered = batch.events.iter().copied().filter(|event| event.0 % 2 == 0).collect::<Vec<_>>();
            apply_inner.apply_event_batch(EventBatch { snapshot_id: batch.snapshot_id, time: batch.time, events: &filtered });
        }
    }

    struct SkipInner;

    impl ApplyWrapper<TestSnapshot, TestEvent> for SkipInner {
        fn apply_event_batch(&mut self, _batch: EventBatch<'_, i64, TestEvent>, _apply_inner: &mut ApplyInner<'_, TestSnapshot>) {}
    }

    fn events(count: usize) -> Vec<TestEvent> {
        (0..count).map(|value| TestEvent(value as i64)).collect()
    }

    #[test]
    fn the_default_wrapper_applies_the_complete_canonical_batch_once() {
        let events = events(3);
        let references = events.iter().collect::<Vec<_>>();
        let mut snapshot = TestSnapshot::default();

        apply(&mut snapshot, EventBatch { snapshot_id: 7, time: 10, events: &references }, 13, &mut ());

        assert_eq!(snapshot.sum, 3);
        assert_eq!(snapshot.time, 10);
        assert_eq!(snapshot.batch_sizes, vec![3]);
        assert_eq!(snapshot.history_counts, vec![13]);
    }

    #[test]
    fn a_wrapper_can_filter_the_effective_batch_without_changing_the_raw_count() {
        let events = events(5);
        let references = events.iter().collect::<Vec<_>>();
        let mut snapshot = TestSnapshot::default();

        apply(&mut snapshot, EventBatch { snapshot_id: 7, time: 10, events: &references }, 25, &mut FilterEven);

        assert_eq!(snapshot.sum, 6);
        assert_eq!(snapshot.batch_sizes, vec![3]);
        assert_eq!(snapshot.history_counts, vec![25]);
    }

    #[test]
    #[should_panic(expected = "an apply wrapper must call the inner apply")]
    fn a_wrapper_must_invoke_the_inner_apply() {
        let events = events(1);
        let references = events.iter().collect::<Vec<_>>();
        let mut snapshot = TestSnapshot::default();

        apply(&mut snapshot, EventBatch { snapshot_id: 7, time: 10, events: &references }, 1, &mut SkipInner);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_apply() {
        let events = events(1_000);
        let references = events.iter().collect::<Vec<_>>();
        let mut criterion = Criterion::default();

        criterion.bench_function("checkpoints/apply/1000_events/one_batch", |bencher| {
            bencher.iter(|| {
                let mut snapshot = TestSnapshot::default();
                apply(&mut snapshot, EventBatch { snapshot_id: 7, time: 10, events: black_box(&references) }, 1_000, &mut ());
                black_box(snapshot)
            });
        });

        criterion.final_summary();
    }
}
