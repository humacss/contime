use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use contime_memory::{ConservativeTrackedSize, SizeDelta, TrackedArc, TrackedBox, TrackedMemoryBudget, TrackedSizeDelta};

#[derive(Clone, Default)]
struct TestBudget(Arc<AtomicIsize>);

impl TestBudget {
    fn used(&self) -> isize {
        self.0.load(Ordering::Relaxed)
    }
}

impl TrackedMemoryBudget for TestBudget {
    fn apply_delta(&self, delta: SizeDelta) {
        let change = match delta {
            SizeDelta::Increase(bytes) => bytes as isize,
            SizeDelta::Decrease(bytes) => -(bytes as isize),
            SizeDelta::Unchanged => 0,
        };
        self.0.fetch_add(change, Ordering::Relaxed);
    }

    fn has_buffer(&self) -> bool {
        true
    }

    fn buffer_size(&self) -> usize {
        usize::MAX
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Value(Vec<u8>);

impl ConservativeTrackedSize for Value {
    fn conservative_tracked_size(&self) -> usize {
        size_of::<Self>().saturating_add(self.0.capacity())
    }
}

impl TrackedSizeDelta for Value {
    fn size_delta<R>(&mut self, action: impl FnOnce(&mut Self) -> R) -> (R, SizeDelta) {
        let before = self.conservative_tracked_size();
        let result = action(self);
        let after = self.conservative_tracked_size();
        let delta = match after.cmp(&before) {
            std::cmp::Ordering::Greater => SizeDelta::Increase(after - before),
            std::cmp::Ordering::Less => SizeDelta::Decrease(before - after),
            std::cmp::Ordering::Equal => SizeDelta::Unchanged,
        };
        (result, delta)
    }
}

#[test]
fn tracked_arc_preserves_standard_arc_sharing_semantics() {
    let standard = Arc::new(Value(vec![1, 2, 3]));
    let standard_clone = standard.clone();
    assert!(Arc::ptr_eq(&standard, &standard_clone));

    let budget = TestBudget::default();
    let tracked = TrackedArc::new(Value(vec![1, 2, 3]), budget.clone());
    let tracked_clone = tracked.clone();
    assert!(std::ptr::eq(tracked.as_ref(), tracked_clone.as_ref()));
    assert_eq!(*tracked, *standard);

    drop(tracked_clone);
    assert!(budget.used() > 0);
    drop(tracked);
    assert_eq!(budget.used(), 0);
}

#[test]
fn tracked_box_preserves_standard_box_deep_clone_semantics() {
    let standard = Box::new(Value(vec![1]));
    let mut standard_clone = standard.clone();
    standard_clone.0.push(2);
    assert_eq!(standard.0, vec![1]);
    assert_eq!(standard_clone.0, vec![1, 2]);

    let budget = TestBudget::default();
    let tracked = TrackedBox::new(Value(vec![1]), budget.clone());
    let mut tracked_clone = tracked.clone();
    tracked_clone.update(|value| value.0.push(2));
    assert_eq!(tracked.0, standard.0);
    assert_eq!(tracked_clone.0, standard_clone.0);

    drop(tracked_clone);
    assert!(budget.used() > 0);
    drop(tracked);
    assert_eq!(budget.used(), 0);
}
