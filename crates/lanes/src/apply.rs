use crate::types::SnapshotOutput;
use crate::{ApplyBatch, ApplyEvents, ApplyLanes, EventFilter, FilterBatch, FilterOutput, Lanes};

impl<S, T, A> FilterOutput<A> for SnapshotOutput<'_, S, T>
where
    A: ApplyLanes,
    S: ApplyEvents<T, A>,
{
    fn output<'a>(&mut self, events: A::Batch<'a>)
    where
        A: 'a,
    {
        assert_eq!(self.output_count, 0, "an event filter must output exactly one apply batch");
        let time = self.time.take().expect("filter output time must be available before application");
        self.snapshot.apply_events(ApplyBatch {
            snapshot_id: self.snapshot_id,
            time,
            history_event_count: self.history_event_count,
            events,
        });
        self.output_count += 1;
    }
}

/// Filters one complete snapshot batch and immediately applies its transient
/// output lane type to the snapshot.
pub fn apply<'a, S, T, F, A, E>(snapshot: &mut S, event_filter: &E, batch: FilterBatch<T, F::Batch<'a>>)
where
    F: Lanes + 'a,
    A: ApplyLanes + 'a,
    E: EventFilter<T, F, A>,
    S: ApplyEvents<T, A>,
    T: Clone,
{
    let mut output = SnapshotOutput {
        snapshot,
        snapshot_id: batch.snapshot_id,
        time: Some(batch.time.clone()),
        history_event_count: batch.history_event_count,
        output_count: 0,
    };
    event_filter.filter(batch, &mut output);
    assert_eq!(output.output_count, 1, "an event filter must output exactly one apply batch");
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use criterion::Criterion;

    use crate::{apply, ApplyBatch, ApplyEvents, ApplyLanes, EventFilter, FilterBatch, FilterLanes, FilterOutput, Lanes, RawBatch};

    struct Attack {
        damage: i64,
    }

    struct Claim {
        bonus: i64,
    }

    enum RawEvent {
        Attack(Attack),
        Claim(Claim),
    }

    #[derive(Clone, Copy)]
    enum CombatFilterEvent<'a> {
        Attack(&'a Attack),
        Claim(&'a Claim),
    }

    struct CombatFilterLanes;

    type CombatFilterBatch<'a> =
        std::iter::Map<std::iter::Copied<std::slice::Iter<'a, &'a RawEvent>>, fn(&'a RawEvent) -> CombatFilterEvent<'a>>;

    impl Lanes for CombatFilterLanes {
        type Event<'a> = CombatFilterEvent<'a>;
        type Batch<'a> = CombatFilterBatch<'a>;
    }

    impl FilterLanes<RawEvent> for CombatFilterLanes {
        fn project<'a>(events: &'a [&'a RawEvent]) -> Self::Batch<'a> {
            events.iter().copied().map(project_combat_event)
        }
    }

    impl ApplyLanes for CombatFilterLanes {}

    fn project_combat_event(event: &RawEvent) -> CombatFilterEvent<'_> {
        match event {
            RawEvent::Attack(event) => CombatFilterEvent::Attack(event),
            RawEvent::Claim(event) => CombatFilterEvent::Claim(event),
        }
    }

    #[derive(Clone, Copy)]
    struct AuthorizedAttack {
        damage: i64,
    }

    #[derive(Clone, Copy)]
    enum CombatApplyEvent {
        AuthorizedAttack(AuthorizedAttack),
    }

    struct CombatApplyLanes;

    impl Lanes for CombatApplyLanes {
        type Event<'a> = CombatApplyEvent;
        type Batch<'a> = Vec<CombatApplyEvent>;
    }

    impl ApplyLanes for CombatApplyLanes {}

    struct AuthorizeAttacks;

    impl EventFilter<i64, CombatFilterLanes, CombatApplyLanes> for AuthorizeAttacks {
        fn filter<'a, O>(&self, batch: FilterBatch<i64, CombatFilterBatch<'a>>, output: &mut O)
        where
            CombatFilterLanes: 'a,
            CombatApplyLanes: 'a,
            O: FilterOutput<CombatApplyLanes>,
        {
            let bonus = batch
                .events
                .clone()
                .find_map(|event| match event {
                    CombatFilterEvent::Claim(event) => Some(event.bonus),
                    CombatFilterEvent::Attack(_) => None,
                })
                .unwrap_or_default();
            let events = batch
                .events
                .filter_map(|event| match event {
                    CombatFilterEvent::Attack(event) => {
                        Some(CombatApplyEvent::AuthorizedAttack(AuthorizedAttack { damage: event.damage + bonus }))
                    }
                    CombatFilterEvent::Claim(_) => None,
                })
                .collect();
            output.output(events);
        }
    }

    #[derive(Default)]
    struct CombatSnapshot {
        snapshot_id: u128,
        time: i64,
        damage: i64,
        applied_history_count: u64,
    }

    impl ApplyEvents<i64, CombatApplyLanes> for CombatSnapshot {
        fn apply_events<'a>(&mut self, batch: ApplyBatch<i64, Vec<CombatApplyEvent>>)
        where
            CombatApplyLanes: 'a,
        {
            self.snapshot_id = batch.snapshot_id;
            self.time = batch.time;
            self.applied_history_count = batch.history_event_count;
            for event in batch.events {
                match event {
                    CombatApplyEvent::AuthorizedAttack(event) => self.damage += event.damage,
                }
            }
        }
    }

    #[derive(Default)]
    struct RawSnapshot {
        applied: usize,
    }

    impl ApplyEvents<i64, CombatFilterLanes> for RawSnapshot {
        fn apply_events<'a>(&mut self, batch: ApplyBatch<i64, CombatFilterBatch<'a>>)
        where
            CombatFilterLanes: 'a,
        {
            self.applied += batch.events.map(black_box).count();
        }
    }

    #[test]
    fn filter_output_lane_type_is_delivered_to_snapshot_apply() {
        let attack = RawEvent::Attack(Attack { damage: 8 });
        let claim = RawEvent::Claim(Claim { bonus: 3 });
        let raw_events = [&attack, &claim];
        let filter_batch = crate::project::<CombatFilterLanes, _, _>(RawBatch {
            snapshot_id: 7,
            time: 11_i64,
            history_event_count: 2,
            events: &raw_events,
        });
        let mut snapshot = CombatSnapshot::default();

        apply::<_, _, CombatFilterLanes, CombatApplyLanes, _>(&mut snapshot, &AuthorizeAttacks, filter_batch);

        assert_eq!(snapshot.snapshot_id, 7);
        assert_eq!(snapshot.time, 11);
        assert_eq!(snapshot.damage, 11);
        assert_eq!(snapshot.applied_history_count, 2);
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_apply() {
        let raw = std::iter::once(RawEvent::Claim(Claim { bonus: 3 }))
            .chain((1..1_000).map(|_| RawEvent::Attack(Attack { damage: 8 })))
            .collect::<Vec<_>>();
        let raw_events = raw.iter().collect::<Vec<_>>();
        let mut criterion = Criterion::default();

        criterion.bench_function("lanes/apply/default_pipeline_1000_events", |bencher| {
            bencher.iter(|| {
                let filter_batch = crate::project::<CombatFilterLanes, _, _>(RawBatch {
                    snapshot_id: 7,
                    time: 11_i64,
                    history_event_count: 1_000,
                    events: black_box(&raw_events),
                });
                let mut snapshot = RawSnapshot::default();
                apply::<_, _, CombatFilterLanes, CombatFilterLanes, _>(&mut snapshot, &crate::PassThrough, filter_batch);
                black_box(snapshot)
            });
        });

        criterion.bench_function("lanes/apply/decorated_pipeline_1000_raw_events", |bencher| {
            bencher.iter(|| {
                let filter_batch = crate::project::<CombatFilterLanes, _, _>(RawBatch {
                    snapshot_id: 7,
                    time: 11_i64,
                    history_event_count: 1_000,
                    events: black_box(&raw_events),
                });
                let mut snapshot = CombatSnapshot::default();
                apply::<_, _, CombatFilterLanes, CombatApplyLanes, _>(&mut snapshot, &AuthorizeAttacks, filter_batch);
                black_box(snapshot)
            });
        });

        criterion.final_summary();
    }
}
