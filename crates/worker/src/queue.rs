use std::hash::Hash;

use priority_queue::PriorityQueue;

/// A persistent keyed priority queue.
///
/// Registered keys are reprioritized rather than removed, so the underlying
/// heap and key index keep their allocation across scheduling cycles.
pub(crate) struct Queue<K, P> {
    queue: PriorityQueue<K, P>,
}

impl<K, P> Queue<K, P>
where
    K: Hash + Eq,
    P: Ord,
{
    #[inline(always)]
    pub(crate) fn new() -> Self {
        Self { queue: PriorityQueue::new() }
    }

    #[inline(always)]
    pub(crate) fn set(&mut self, key: K, priority: P) -> Option<P> {
        self.queue.push(key, priority)
    }

    #[inline(always)]
    pub(crate) fn change_priority(&mut self, key: &K, priority: P) -> Option<P> {
        self.queue.change_priority(key, priority)
    }

    #[inline(always)]
    pub(crate) fn pop(&mut self) -> Option<(K, P)> {
        self.queue.pop()
    }

    #[inline(always)]
    pub(crate) fn clear(&mut self) {
        self.queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    use criterion::Criterion;

    use super::Queue;

    #[test]
    fn changing_priority_changes_which_key_is_highest() {
        let mut queue = Queue::new();
        queue.set(7_u128, 10_u64);
        queue.set(9_u128, 20_u64);
        queue.change_priority(&7, 30);

        assert_eq!(queue.pop(), Some((7, 30)));
    }

    #[test]
    fn pop_removes_the_highest_priority() {
        let mut queue = Queue::new();
        queue.set(7_u128, 10_u64);
        queue.set(9_u128, 20_u64);

        assert_eq!(queue.pop(), Some((9, 20)));
        assert_eq!(queue.pop(), Some((7, 10)));
    }

    #[test]
    fn setting_an_existing_key_updates_instead_of_registering_twice() {
        let mut queue = Queue::new();
        queue.set(7_u128, 10_u64);

        assert_eq!(queue.set(7, 20), Some(10));
        assert_eq!(queue.pop(), Some((7, 20)));
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_queue() {
        let mut criterion = Criterion::default();

        criterion.bench_function("worker/queue/1000_registrations", |bencher| {
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let mut queue = Queue::new();
                    let started = Instant::now();
                    for key in 0..1_000_u128 {
                        black_box(queue.set(key, key));
                    }
                    measured += started.elapsed();
                    black_box(queue);
                }
                measured
            });
        });

        criterion.bench_function("worker/queue/1000_priority_changes", |bencher| {
            let mut queue = Queue::new();
            for key in 0..1_000_u128 {
                queue.set(key, key);
            }
            bencher.iter(|| {
                for key in 0..1_000_u128 {
                    black_box(queue.change_priority(&key, black_box(key + 1)));
                }
            });
        });

        criterion.bench_function("worker/queue/1000_pops", |bencher| {
            let mut queue = Queue::new();
            for key in 0..1_000_u128 {
                queue.set(key, key + 1);
            }
            bencher.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let started = Instant::now();
                    for _ in 0..1_000 {
                        black_box(queue.pop());
                    }
                    measured += started.elapsed();
                    for key in 0..1_000_u128 {
                        queue.set(key, key + 1);
                    }
                }
                measured
            });
        });

        criterion.final_summary();
    }
}
