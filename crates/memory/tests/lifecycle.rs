use std::mem::size_of;
use std::sync::mpsc;

use contime_memory::{
    AtomicMemoryBudget, CachedAccount, ConservativeTrackedSize, MemoryBudget, MemoryBudgetConfig, MemoryStatus, TrackedArc, TrackedBox,
};

#[derive(Debug)]
struct Event(Vec<u8>);

impl ConservativeTrackedSize for Event {
    fn conservative_tracked_size(&self) -> usize {
        size_of::<Self>() + self.0.capacity()
    }
}

struct Message {
    event: TrackedArc<Event>,
    label: String,
}

impl ConservativeTrackedSize for Message {
    fn conservative_tracked_size(&self) -> usize {
        // `event` accounts for its allocation and this handle separately.
        size_of::<String>() + self.label.capacity()
    }
}

#[derive(Clone)]
struct Snapshot(Vec<u8>);

impl ConservativeTrackedSize for Snapshot {
    fn conservative_tracked_size(&self) -> usize {
        size_of::<Self>() + self.0.capacity()
    }
}

fn budget(limit: usize, actions: usize, buffer: usize) -> AtomicMemoryBudget {
    AtomicMemoryBudget::new(MemoryBudgetConfig { hard_limit: limit, concurrent_actions: actions, action_buffer: buffer }).unwrap()
}

#[test]
fn channel_movement_changes_no_accounting_and_drop_releases_everything() {
    let memory = budget(usize::MAX, 0, usize::MAX);
    let event = TrackedArc::new(Event(vec![7; 64]), memory.clone());
    let message = TrackedBox::new(Message { event: event.clone(), label: "move only".to_owned() }, memory.clone());
    assert_eq!(message.event.0[0], 7);
    let before = memory.state();
    let (sender, receiver) = mpsc::channel();
    sender.send(message).unwrap();
    assert_eq!(memory.state(), before);
    let received = receiver.recv().unwrap();
    assert_eq!(memory.state(), before);
    drop(received);
    assert!(memory.state().used > 0);
    drop(event);
    assert_eq!(memory.state().used, 0);
}

#[test]
fn deep_snapshot_clones_mutate_independently_with_both_accounts() {
    let measured_budget = budget(usize::MAX, 0, usize::MAX);
    let cached_budget = budget(usize::MAX, 0, usize::MAX);
    let mut measured = TrackedBox::new(Snapshot(vec![1]), measured_budget.clone());
    let mut cached = TrackedBox::<Snapshot, CachedAccount, _>::new_with_account(Snapshot(vec![1]), cached_budget.clone());
    let untouched = measured.clone();
    let measured_before = measured_budget.state().allocation_bytes;
    let cached_before = cached_budget.state().allocation_bytes;
    measured.update(|snapshot| snapshot.0.reserve_exact(64));
    cached.update(|snapshot| snapshot.0.reserve_exact(64));
    assert_eq!(untouched.0, vec![1]);
    assert_eq!(measured.0, cached.0);
    assert_eq!(measured_budget.state().allocation_bytes - measured_before, cached_budget.state().allocation_bytes - cached_before);
    drop(untouched);
    drop(measured);
    drop(cached);
    assert_eq!(measured_budget.state().used, 0);
    assert_eq!(cached_budget.state().used, 0);
}

#[test]
fn completed_growth_is_retained_and_reports_threshold_diagnostics() {
    let memory = budget(1_000, 1, 200);
    let mut snapshot = TrackedBox::new(Snapshot(Vec::new()), memory.clone());
    snapshot.update(|snapshot| snapshot.0.reserve_exact(850));
    let state = memory.state();
    assert_eq!(snapshot.0.capacity(), 850);
    assert_eq!(state.status, MemoryStatus::ActionBlocked);
    assert_eq!(state.buffer_exceeded_count, 1);
    snapshot.update(|snapshot| snapshot.0.shrink_to_fit());
    assert_eq!(memory.state().status, MemoryStatus::Ready);
    drop(snapshot);
    assert_eq!(memory.state().used, 0);
}
