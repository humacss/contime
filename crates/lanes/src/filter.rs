use crate::{ApplyLanes, EventFilter, FilterBatch, FilterLanes, FilterOutput, Lanes, RawBatch};

/// Projects one canonical raw batch into a snapshot-specific borrowed filter
/// batch while preserving metadata and canonical order.
pub fn project<'a, L, T, R>(batch: RawBatch<'a, T, R>) -> FilterBatch<T, L::Batch<'a>>
where
    L: FilterLanes<R> + 'a,
{
    FilterBatch {
        snapshot_id: batch.snapshot_id,
        time: batch.time,
        history_event_count: batch.history_event_count,
        events: L::project(batch.events),
    }
}

/// Runs one statically selected snapshot filter.
pub fn filter<'a, T, F, A, E, O>(event_filter: &E, batch: FilterBatch<T, F::Batch<'a>>, output: &mut O)
where
    F: Lanes + 'a,
    A: ApplyLanes + 'a,
    E: EventFilter<T, F, A>,
    O: FilterOutput<A>,
{
    event_filter.filter(batch, output);
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::mem::size_of;
    use std::ptr;

    use criterion::Criterion;

    use crate::{filter, project, ApplyLanes, FilterLanes, FilterOutput, Lanes, PassThrough, RawBatch};

    #[derive(Debug)]
    struct Attack;

    #[derive(Debug)]
    struct Claim;

    enum RawEvent {
        Attack(Attack),
        Claim(Claim),
    }

    #[derive(Clone, Copy, Debug)]
    enum CharacterFilterEvent<'a> {
        Attack(&'a Attack),
        Claim(&'a Claim),
    }

    struct CharacterFilterLanes;

    type CharacterFilterBatch<'a> =
        std::iter::FilterMap<std::iter::Copied<std::slice::Iter<'a, &'a RawEvent>>, fn(&'a RawEvent) -> Option<CharacterFilterEvent<'a>>>;

    impl Lanes for CharacterFilterLanes {
        type Event<'a> = CharacterFilterEvent<'a>;
        type Batch<'a> = CharacterFilterBatch<'a>;
    }

    impl FilterLanes<RawEvent> for CharacterFilterLanes {
        fn project<'a>(events: &'a [&'a RawEvent]) -> Self::Batch<'a> {
            events.iter().copied().filter_map(project_character_event)
        }
    }

    impl ApplyLanes for CharacterFilterLanes {}

    fn project_character_event(event: &RawEvent) -> Option<CharacterFilterEvent<'_>> {
        Some(match event {
            RawEvent::Attack(event) => CharacterFilterEvent::Attack(event),
            RawEvent::Claim(event) => CharacterFilterEvent::Claim(event),
        })
    }

    #[test]
    fn projection_preserves_raw_order_and_borrows_payloads() {
        let attack = RawEvent::Attack(Attack);
        let claim = RawEvent::Claim(Claim);
        let raw_events = [&attack, &claim];

        let projected =
            project::<CharacterFilterLanes, _, _>(RawBatch { snapshot_id: 7, time: 11_i64, history_event_count: 2, events: &raw_events });
        let events = projected.events.collect::<Vec<_>>();

        assert_eq!(projected.snapshot_id, 7);
        assert_eq!(projected.time, 11);
        assert_eq!(projected.history_event_count, 2);
        assert!(
            matches!(events[0], CharacterFilterEvent::Attack(event) if ptr::eq(event, match &attack { RawEvent::Attack(event) => event, _ => unreachable!() }))
        );
        assert!(
            matches!(events[1], CharacterFilterEvent::Claim(event) if ptr::eq(event, match &claim { RawEvent::Claim(event) => event, _ => unreachable!() }))
        );
    }

    #[derive(Default)]
    struct ApplyTrace {
        variants: Vec<&'static str>,
    }

    impl FilterOutput<CharacterFilterLanes> for ApplyTrace {
        fn output<'a>(&mut self, events: CharacterFilterBatch<'a>)
        where
            CharacterFilterLanes: 'a,
        {
            self.variants = events
                .map(|event| match event {
                    CharacterFilterEvent::Attack(_) => "attack",
                    CharacterFilterEvent::Claim(_) => "claim",
                })
                .collect();
        }
    }

    #[test]
    fn pass_through_uses_the_projected_batch_as_the_apply_batch() {
        let attack = RawEvent::Attack(Attack);
        let claim = RawEvent::Claim(Claim);
        let raw_events = [&attack, &claim];
        let projected =
            project::<CharacterFilterLanes, _, _>(RawBatch { snapshot_id: 7, time: 11_i64, history_event_count: 2, events: &raw_events });
        let mut trace = ApplyTrace::default();

        filter::<_, CharacterFilterLanes, CharacterFilterLanes, _, _>(&PassThrough, projected, &mut trace);

        assert_eq!(trace.variants, ["attack", "claim"]);
    }

    #[test]
    fn borrowed_filter_lane_size_is_bounded_by_reference_storage() {
        assert!(size_of::<CharacterFilterEvent<'_>>() <= 2 * size_of::<usize>());
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_filter() {
        let raw =
            (0..1_000).map(|index| if index % 2 == 0 { RawEvent::Attack(Attack) } else { RawEvent::Claim(Claim) }).collect::<Vec<_>>();
        let raw_events = raw.iter().collect::<Vec<_>>();
        let mut criterion = Criterion::default();

        criterion.bench_function("lanes/filter/project_1000_raw_events", |bencher| {
            bencher.iter(|| {
                let projected = project::<CharacterFilterLanes, _, _>(RawBatch {
                    snapshot_id: 7,
                    time: 11_i64,
                    history_event_count: 1_000,
                    events: black_box(&raw_events),
                });
                projected.events.map(black_box).count()
            });
        });

        criterion.final_summary();
    }
}
