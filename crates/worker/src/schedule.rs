use std::cmp::Reverse;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ahash::AHashMap;

use crate::queue::Queue;

type CountPriority = (u64, Reverse<Instant>, Reverse<u128>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SnapshotState {
    actual_count: u64,
    priority_count: u64,
    dirty_since: Instant,
}

#[derive(Clone, Copy)]
struct DeadlineEntry {
    snapshot_id: u128,
    dirty_since: Instant,
}

pub(crate) struct Schedule {
    deadlines: VecDeque<DeadlineEntry>,
    by_count: Queue<u128, CountPriority>,
    states: AHashMap<u128, SnapshotState>,
    active_count: usize,
    deadline_compaction_minimum: usize,
    deadline_compaction_multiplier: usize,
}

impl Schedule {
    pub(crate) fn new(deadline_compaction_minimum: usize, deadline_compaction_multiplier: usize) -> Self {
        Self {
            deadlines: VecDeque::new(),
            by_count: Queue::new(),
            states: AHashMap::new(),
            active_count: 0,
            deadline_compaction_minimum,
            deadline_compaction_multiplier,
        }
    }

    pub(crate) fn is_empty(&mut self) -> bool {
        if self.active_count != 0 {
            return false;
        }

        self.deadlines.clear();
        self.by_count.clear();
        self.states.clear();
        true
    }

    pub(crate) fn mark_dirty(&mut self, snapshot_id: u128, now: Instant) {
        if let Some(state) = self.states.get_mut(&snapshot_id) {
            if state.actual_count != 0 {
                state.actual_count = state.actual_count.saturating_add(1);
                if state.actual_count > state.priority_count {
                    let priority = Self::count_priority(snapshot_id, *state);
                    self.by_count.change_priority(&snapshot_id, priority);
                    state.priority_count = state.actual_count;
                }
                return;
            }

            state.actual_count = 1;
            state.dirty_since = now;
            self.active_count += 1;
            self.deadlines.push_back(DeadlineEntry { snapshot_id, dirty_since: now });
            if state.actual_count > state.priority_count {
                let priority = Self::count_priority(snapshot_id, *state);
                if state.priority_count == 0 {
                    self.by_count.set(snapshot_id, priority);
                } else {
                    self.by_count.change_priority(&snapshot_id, priority);
                }
                state.priority_count = state.actual_count;
            }
            self.compact_deadlines_if_needed();
            return;
        }

        let state = SnapshotState { actual_count: 1, priority_count: 1, dirty_since: now };
        self.states.insert(snapshot_id, state);
        self.active_count += 1;
        self.deadlines.push_back(DeadlineEntry { snapshot_id, dirty_since: now });
        self.by_count.set(snapshot_id, Self::count_priority(snapshot_id, state));
        self.compact_deadlines_if_needed();
    }

    pub(crate) fn next_deadline(&mut self, maximum_dirty_age: Duration) -> Option<Instant> {
        self.discard_stale_deadlines();
        self.deadlines.front()?.dirty_since.checked_add(maximum_dirty_age)
    }

    pub(crate) fn pop_next(&mut self, now: Instant, maximum_dirty_age: Duration) -> Option<u128> {
        if self.oldest_is_due(now, maximum_dirty_age) {
            return self.pop_oldest();
        }
        self.pop_largest(now)
    }

    pub(crate) fn pop_overdue(&mut self, now: Instant, maximum_dirty_age: Duration) -> Option<u128> {
        self.oldest_is_due(now, maximum_dirty_age).then(|| self.pop_oldest()).flatten()
    }

    pub(crate) fn pop_largest(&mut self, _now: Instant) -> Option<u128> {
        loop {
            let (snapshot_id, observed) = self.by_count.pop()?;
            let Some(state) = self.states.get_mut(&snapshot_id) else {
                continue;
            };
            state.priority_count = 0;

            if state.actual_count == 0 {
                continue;
            }

            let actual = Self::count_priority(snapshot_id, *state);
            if observed == actual {
                state.actual_count = 0;
                self.active_count -= 1;
                return Some(snapshot_id);
            }

            state.priority_count = state.actual_count;
            self.by_count.set(snapshot_id, actual);
        }
    }

    fn pop_oldest(&mut self) -> Option<u128> {
        self.discard_stale_deadlines();
        let deadline = self.deadlines.pop_front()?;
        let state = self.states.get_mut(&deadline.snapshot_id)?;
        state.actual_count = 0;
        self.active_count -= 1;
        Some(deadline.snapshot_id)
    }

    fn oldest_is_due(&mut self, now: Instant, maximum_dirty_age: Duration) -> bool {
        self.next_deadline(maximum_dirty_age).is_some_and(|deadline| deadline <= now)
    }

    fn discard_stale_deadlines(&mut self) {
        while let Some(deadline) = self.deadlines.front() {
            let is_current =
                self.state(deadline.snapshot_id).is_some_and(|state| state.actual_count != 0 && state.dirty_since == deadline.dirty_since);
            if is_current {
                break;
            }
            self.deadlines.pop_front();
        }
    }

    fn compact_deadlines_if_needed(&mut self) {
        let deadline_count = self.deadlines.len();
        if deadline_count <= self.deadline_compaction_minimum {
            return;
        }
        if deadline_count <= self.active_count.saturating_mul(self.deadline_compaction_multiplier) {
            return;
        }

        let states = &self.states;
        self.deadlines.retain(|deadline| {
            states.get(&deadline.snapshot_id).is_some_and(|state| state.actual_count != 0 && state.dirty_since == deadline.dirty_since)
        });
    }

    #[inline(always)]
    fn state(&self, snapshot_id: u128) -> Option<SnapshotState> {
        self.states.get(&snapshot_id).copied()
    }

    #[inline(always)]
    fn count_priority(snapshot_id: u128, state: SnapshotState) -> CountPriority {
        (state.actual_count, Reverse(state.dirty_since), Reverse(snapshot_id))
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    use criterion::{BatchSize, Criterion};

    use super::Schedule;

    #[test]
    fn largest_pending_snapshot_wins_before_a_deadline() {
        let now = Instant::now();
        let mut schedule = Schedule::new(usize::MAX, 2);
        schedule.mark_dirty(1, now);
        schedule.mark_dirty(2, now);
        schedule.mark_dirty(2, now);

        assert_eq!(schedule.pop_next(now, Duration::from_micros(100)), Some(2));
    }

    #[test]
    fn overdue_snapshot_wins_even_with_less_pending_work() {
        let start = Instant::now();
        let mut schedule = Schedule::new(usize::MAX, 2);
        schedule.mark_dirty(1, start);
        schedule.mark_dirty(2, start + Duration::from_micros(50));
        schedule.mark_dirty(2, start + Duration::from_micros(50));

        assert_eq!(schedule.pop_next(start + Duration::from_micros(100), Duration::from_micros(100)), Some(1));
    }

    #[test]
    fn all_snapshots_past_the_same_deadline_can_be_removed() {
        let start = Instant::now();
        let mut schedule = Schedule::new(usize::MAX, 2);
        schedule.mark_dirty(1, start);
        schedule.mark_dirty(2, start);
        let now = start + Duration::from_micros(100);

        assert!(schedule.pop_overdue(now, Duration::from_micros(100)).is_some());
        assert!(schedule.pop_overdue(now, Duration::from_micros(100)).is_some());
        assert!(schedule.is_empty());
    }

    #[test]
    fn processed_snapshot_does_not_become_overdue_while_inactive() {
        let start = Instant::now();
        let mut schedule = Schedule::new(usize::MAX, 2);
        schedule.mark_dirty(7, start);
        assert_eq!(schedule.pop_largest(start), Some(7));

        assert_eq!(schedule.pop_overdue(start + Duration::from_secs(1), Duration::from_micros(100)), None);
    }

    #[test]
    fn reactivated_snapshot_uses_its_new_dirty_time() {
        let start = Instant::now();
        let reactivated = start + Duration::from_millis(1);
        let mut schedule = Schedule::new(usize::MAX, 2);
        schedule.mark_dirty(7, start);
        schedule.pop_largest(start);
        schedule.mark_dirty(7, reactivated);

        assert_eq!(schedule.next_deadline(Duration::from_micros(100)), reactivated.checked_add(Duration::from_micros(100)));
    }

    #[test]
    fn deadline_selection_removes_the_snapshot_from_count_scheduling() {
        let start = Instant::now();
        let mut schedule = Schedule::new(usize::MAX, 2);
        schedule.mark_dirty(1, start);
        schedule.mark_dirty(1, start);
        schedule.mark_dirty(2, start + Duration::from_micros(50));

        assert_eq!(schedule.pop_overdue(start + Duration::from_micros(100), Duration::from_micros(100)), Some(1));
        assert_eq!(schedule.pop_largest(start + Duration::from_micros(100)), Some(2));
    }

    #[test]
    fn count_priority_is_only_updated_after_actual_count_exceeds_it() {
        let start = Instant::now();
        let mut schedule = Schedule::new(usize::MAX, 2);
        for _ in 0..5 {
            schedule.mark_dirty(1, start);
        }
        schedule.mark_dirty(2, start + Duration::from_micros(50));

        assert_eq!(schedule.pop_overdue(start + Duration::from_micros(100), Duration::from_micros(100),), Some(1),);

        let reactivated = start + Duration::from_micros(200);
        for _ in 0..4 {
            schedule.mark_dirty(1, reactivated);
        }
        assert_eq!(schedule.states[&1].actual_count, 4);
        assert_eq!(schedule.states[&1].priority_count, 5);

        schedule.mark_dirty(1, reactivated);
        assert_eq!(schedule.states[&1].actual_count, 5);
        assert_eq!(schedule.states[&1].priority_count, 5);

        schedule.mark_dirty(1, reactivated);
        assert_eq!(schedule.states[&1].actual_count, 6);
        assert_eq!(schedule.states[&1].priority_count, 6);
        assert_eq!(schedule.pop_largest(reactivated), Some(1));
    }

    #[test]
    fn stale_priority_is_repaired_without_losing_reactivated_work() {
        let start = Instant::now();
        let mut schedule = Schedule::new(usize::MAX, 2);
        for _ in 0..5 {
            schedule.mark_dirty(1, start);
        }
        assert_eq!(schedule.pop_overdue(start + Duration::from_micros(100), Duration::from_micros(100),), Some(1),);

        let reactivated = start + Duration::from_millis(1);
        schedule.mark_dirty(1, reactivated);
        schedule.mark_dirty(2, reactivated);
        schedule.mark_dirty(2, reactivated);

        assert_eq!(schedule.pop_largest(reactivated), Some(2));
        assert_eq!(schedule.pop_largest(reactivated), Some(1));
    }

    #[test]
    fn deadline_compaction_removes_stale_entries_after_the_configured_threshold() {
        let start = Instant::now();
        let mut schedule = Schedule::new(4, 2);

        for offset in 0..5 {
            let now = start + Duration::from_micros(offset);
            schedule.mark_dirty(7, now);
            if offset < 4 {
                assert_eq!(schedule.pop_largest(now), Some(7));
            }
        }

        assert_eq!(schedule.deadlines.len(), 1);
        assert_eq!(schedule.next_deadline(Duration::from_micros(100)), start.checked_add(Duration::from_micros(104)));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_schedule() {
        let mut criterion = Criterion::default();

        criterion.bench_function("worker/schedule/1000_dirty_activations", |bencher| {
            let now = Instant::now();
            let mut schedule = Schedule::new(usize::MAX, 2);
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let started = Instant::now();
                    for snapshot_id in 0..1_000_u128 {
                        schedule.mark_dirty(snapshot_id, now);
                    }
                    measured += started.elapsed();
                    while schedule.pop_largest(now).is_some() {}
                    schedule.is_empty();
                }
                measured
            });
        });

        criterion.bench_function("worker/schedule/1000_existing_dirty_updates", |bencher| {
            let now = Instant::now();
            let mut schedule = Schedule::new(usize::MAX, 2);
            for snapshot_id in 0..1_000_u128 {
                schedule.mark_dirty(snapshot_id, now);
            }
            bencher.iter(|| {
                for snapshot_id in 0..1_000_u128 {
                    schedule.mark_dirty(snapshot_id, now);
                }
            });
        });

        criterion.bench_function("worker/schedule/1000_updates_below_priority_count", |bencher| {
            let initial = Instant::now();
            let initial_due = initial + Duration::from_micros(100);
            let reactivated = initial + Duration::from_millis(1);
            let reactivated_due = reactivated + Duration::from_micros(100);
            let mut schedule = Schedule::new(usize::MAX, 2);
            for snapshot_id in 0..1_000_u128 {
                for _ in 0..5 {
                    schedule.mark_dirty(snapshot_id, initial);
                }
            }
            while schedule.pop_overdue(initial_due, Duration::from_micros(100)).is_some() {}

            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let started = Instant::now();
                    for snapshot_id in 0..1_000_u128 {
                        schedule.mark_dirty(snapshot_id, reactivated);
                    }
                    measured += started.elapsed();
                    while schedule.pop_overdue(reactivated_due, Duration::from_micros(100)).is_some() {}
                }
                measured
            });
        });

        criterion.bench_function("worker/schedule/1000_count_selected", |bencher| {
            let now = Instant::now();
            let mut schedule = Schedule::new(usize::MAX, 2);
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    for snapshot_id in 0..1_000_u128 {
                        schedule.mark_dirty(snapshot_id, now);
                    }
                    let started = Instant::now();
                    while let Some(snapshot_id) = schedule.pop_largest(now) {
                        black_box(snapshot_id);
                    }
                    schedule.is_empty();
                    measured += started.elapsed();
                }
                measured
            });
        });

        criterion.bench_function("worker/schedule/1000_deadline_lookups", |bencher| {
            let now = Instant::now();
            let mut schedule = Schedule::new(usize::MAX, 2);
            schedule.mark_dirty(7, now);
            bencher.iter(|| {
                for _ in 0..1_000 {
                    black_box(schedule.next_deadline(Duration::from_micros(100)));
                }
            });
        });

        criterion.bench_function("worker/schedule/1000_deadline_selected", |bencher| {
            let now = Instant::now();
            let due = now + Duration::from_micros(100);
            let mut schedule = Schedule::new(usize::MAX, 2);
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    for snapshot_id in 0..1_000_u128 {
                        schedule.mark_dirty(snapshot_id, now);
                    }
                    let started = Instant::now();
                    while let Some(snapshot_id) = schedule.pop_overdue(due, Duration::from_micros(100)) {
                        black_box(snapshot_id);
                    }
                    schedule.is_empty();
                    measured += started.elapsed();
                }
                measured
            });
        });

        criterion.bench_function("worker/schedule/1000_stale_deadline_cleanup", |bencher| {
            bencher.iter_batched(
                || {
                    let now = Instant::now();
                    let mut schedule = Schedule::new(usize::MAX, 2);
                    for snapshot_id in 0..1_000_u128 {
                        schedule.mark_dirty(snapshot_id, now);
                    }
                    while schedule.pop_largest(now).is_some() {}
                    schedule
                },
                |mut schedule| black_box(schedule.next_deadline(Duration::from_micros(100))),
                BatchSize::LargeInput,
            );
        });

        criterion.bench_function("worker/schedule/1000_stale_count_cleanup", |bencher| {
            bencher.iter_batched(
                || {
                    let now = Instant::now();
                    let due = now + Duration::from_micros(100);
                    let mut schedule = Schedule::new(usize::MAX, 2);
                    for snapshot_id in 0..1_000_u128 {
                        schedule.mark_dirty(snapshot_id, now);
                    }
                    while schedule.pop_overdue(due, Duration::from_micros(100)).is_some() {}
                    schedule.mark_dirty(10_000, now + Duration::from_millis(1));
                    schedule
                },
                |mut schedule| black_box(schedule.pop_largest(Instant::now())),
                BatchSize::LargeInput,
            );
        });

        criterion.bench_function("worker/schedule/2001_to_1000_deadline_compaction", |bencher| {
            bencher.iter_batched(
                || {
                    let stale = Instant::now();
                    let current = stale + Duration::from_micros(1);
                    let mut schedule = Schedule::new(0, 1);
                    schedule.active_count = 1_000;
                    for snapshot_id in 0..1_000_u128 {
                        schedule
                            .states
                            .insert(snapshot_id, super::SnapshotState { actual_count: 1, priority_count: 0, dirty_since: current });
                    }
                    for snapshot_id in 0..1_001_u128 {
                        schedule.deadlines.push_back(super::DeadlineEntry { snapshot_id: snapshot_id % 1_000, dirty_since: stale });
                    }
                    for snapshot_id in 0..1_000_u128 {
                        schedule.deadlines.push_back(super::DeadlineEntry { snapshot_id, dirty_since: current });
                    }
                    schedule
                },
                |mut schedule| {
                    schedule.compact_deadlines_if_needed();
                    black_box(schedule.deadlines.len())
                },
                BatchSize::LargeInput,
            );
        });

        for minimum in [64, 256, 1_024] {
            criterion.bench_function(&format!("worker/schedule/1000_deadline_churn/minimum_{minimum}"), |bencher| {
                bencher.iter_batched(
                    || (Schedule::new(minimum, 2), Instant::now()),
                    |(mut schedule, start)| {
                        for offset in 0..1_000_u64 {
                            let now = start + Duration::from_nanos(offset);
                            schedule.mark_dirty(7, now);
                            black_box(schedule.pop_largest(now));
                        }
                        black_box(schedule.deadlines.len())
                    },
                    BatchSize::LargeInput,
                );
            });
        }

        criterion.final_summary();
    }
}
