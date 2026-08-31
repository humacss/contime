use std::fmt;
use std::ops::Deref;

use contime_api::RejectionMessage;
use contime_memory::TrackedArc;

use crate::{Input, MemoryBudget, RejectionReason, TrackedEvent};

pub(crate) fn prepare_inputs<I>(
    budget: &MemoryBudget,
    inputs: Vec<I>,
) -> Result<Vec<TrackedEvent<I>>, Vec<RejectionMessage<RejectionReason>>>
where
    I: Input,
{
    let retained = inputs.iter().fold(0_usize, |total, input| total.saturating_add(input.conservative_tracked_size()));
    if !budget.can_admit(retained) {
        return Err(inputs
            .iter()
            .map(|input| RejectionMessage { event_id: input.event_id(), reason: RejectionReason::MemoryFull })
            .collect());
    }

    Ok(inputs.into_iter().map(|input| TrackedEvent { inner: TrackedArc::new(input, budget.clone()) }).collect())
}

impl<I> Clone for TrackedEvent<I>
where
    I: Input,
{
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<I> Deref for TrackedEvent<I>
where
    I: Input,
{
    type Target = I;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<I> fmt::Debug for TrackedEvent<I>
where
    I: Input + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<I> AsRef<I> for TrackedEvent<I>
where
    I: Input,
{
    fn as_ref(&self) -> &I {
        self
    }
}

impl<I> contime_events::Event for TrackedEvent<I>
where
    I: Input,
{
    type Time = I::Time;

    fn event_id(&self) -> u128 {
        self.deref().event_id()
    }

    fn time(&self) -> Self::Time {
        self.deref().time()
    }
}

impl<I> contime_router::RoutableInput for TrackedEvent<I>
where
    I: Input,
{
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        self.deref().snapshot_ids(emit);
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use contime_memory::ConservativeTrackedSize;
    use criterion::{BatchSize, Criterion};

    use super::prepare_inputs;
    use crate::{Input, MemoryBudget, RejectionReason};

    #[derive(Debug)]
    struct TestInput {
        id: u128,
        time: i64,
        snapshots: Vec<u128>,
        retained: usize,
    }

    impl ConservativeTrackedSize for TestInput {
        fn conservative_tracked_size(&self) -> usize {
            self.retained
        }
    }

    impl Input for TestInput {
        type Time = i64;

        fn event_id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> Self::Time {
            self.time
        }

        fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
            self.snapshots.iter().copied().for_each(emit);
        }
    }

    fn input(id: u128, retained: usize) -> TestInput {
        TestInput { id, time: id as i64, snapshots: vec![7], retained }
    }

    #[test]
    fn accepted_inputs_are_tracked_once_and_clones_only_add_pointer_memory() {
        let budget = MemoryBudget::new(10_000, 1_000);
        let events = prepare_inputs(&budget, vec![input(1, 64), input(2, 64)]).unwrap();
        let after_wrap = budget.used();

        let clone = events[0].clone();
        assert_eq!(budget.used() - after_wrap, std::mem::size_of_val(&clone));

        drop(clone);
        drop(events);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn an_over_budget_batch_is_rejected_before_any_event_is_wrapped() {
        let budget = MemoryBudget::new(200, 100);

        let rejection = prepare_inputs(&budget, vec![input(3, 60), input(4, 60)]).unwrap_err();

        assert_eq!(rejection.len(), 2);
        assert_eq!(rejection[0].event_id, 3);
        assert_eq!(rejection[0].reason, RejectionReason::MemoryFull);
        assert_eq!(rejection[1].event_id, 4);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_input() {
        let mut criterion = Criterion::default();
        criterion.bench_function("core/input/prepare_1000", |bencher| {
            bencher.iter_batched(
                || (MemoryBudget::new(usize::MAX, 0), (0..1_000).map(|id| input(id, 64)).collect::<Vec<_>>()),
                |(budget, inputs)| black_box(prepare_inputs(&budget, inputs).unwrap()),
                BatchSize::LargeInput,
            );
        });
        criterion.final_summary();
    }
}
