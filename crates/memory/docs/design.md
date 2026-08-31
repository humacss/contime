# Minimal Tracked Memory Design

## Purpose

`contime-memory` provides ownership types that replace ordinary `Arc` and
`Box` where ConTime must account for retained memory automatically. The crate
defines the compile-time contracts those wrappers need, but it does not choose
how a runtime stores counters, enforces admission, reports diagnostics, or
coordinates a memory process.

Core owns runtime policy. The memory crate owns lifecycle behavior: measuring
allocations and applying the correct deltas on creation, clone, mutation, and
drop.

## Public contract

The complete contract consists of one enum and three traits.

```rust
pub enum SizeDelta {
    Increase(usize),
    Decrease(usize),
    Unchanged,
}

pub trait ConservativeTrackedSize {
    fn conservative_tracked_size(&self) -> usize;
}

pub trait TrackedSizeDelta {
    fn size_delta<R>(
        &mut self,
        action: impl FnOnce(&mut Self) -> R,
    ) -> (R, SizeDelta);
}

pub trait TrackedMemoryBudget: Clone {
    fn apply_delta(&self, delta: SizeDelta);
    fn has_buffer(&self) -> bool;
    fn buffer_size(&self) -> usize;
}
```

`ConservativeTrackedSize` measures the largest reasonable amount of memory
currently retained by the underlying value. It includes inline storage and
retained capacities. Nested tracked wrappers exclude their own memory because
they report it independently.

`TrackedSizeDelta` is implemented by a mutable tracked value. The value owns
its measurement strategy and any cached sizing state. It runs the provided
action and returns both the domain result and the resulting `SizeDelta`.
Implementations may measure twice, compare against cached state, or derive the
delta directly.

`TrackedMemoryBudget` is a reporting and query boundary. An implementation may
use atomics, thread-local counters, channels, intermediate aggregators, or a
dedicated memory process. `has_buffer` reports whether the configured action
buffer remains available. `buffer_size` exposes that allowance for diagnostics.

The crate contains no production implementation of these traits. Concrete
implementations in this crate exist only as unit-test and benchmark fixtures.

## Tracked Arc

`TrackedArc<T, B>` replaces `Arc<T>`, where `T: ConservativeTrackedSize` and
`B: TrackedMemoryBudget`.

- Creation measures and reports the shared allocation plus its first handle.
- Ordinary `Clone` reports one additional handle before cloning the Arc.
- Every handle drop releases its handle size.
- The final shared-allocation drop measures and releases the allocation once.
- The public wrapper remains exactly one machine pointer wide.
- Access is immutable.

The wrapper records completed reality. It does not reject construction or
cloning based on `has_buffer`; the orchestrator consults the budget before
starting memory-growing work.

## Tracked Box

`TrackedBox<T, B>` replaces `Box<T>`, where `T: ConservativeTrackedSize` and
`B: TrackedMemoryBudget`. Mutation additionally requires
`T: TrackedSizeDelta`.

Its constructor is `TrackedBox::new(value, budget)`.

- Creation reports its allocation plus handle.
- Ordinary `Clone` deeply clones `T` and reports an independent allocation.
- Any cached delta state belongs to `T` and follows `T`'s clone semantics.
- Mutation is available only through `update`.
- `update` calls `T::size_delta`, applies the returned delta to the budget, and
  returns the closure result.
- Drop releases the allocation and handle.
- The public wrapper remains exactly one machine pointer wide.
- There is no `DerefMut` or `into_inner` escape hatch.

## Runtime boundary

Core may initially implement `TrackedMemoryBudget` with an `Arc<AtomicUsize>`.
A future reporting thread or hierarchical counter is an alternate
implementation of the same trait, not a change to the tracked ownership types.

## Source layout

Production code has four files:

- `lib.rs`: module declarations and public re-exports only.
- `types.rs`: `SizeDelta`, the three traits, wrapper declarations, and private
  allocation types.
- `tracked_arc.rs`: all Arc replacement behavior.
- `tracked_box.rs`: all Box replacement behavior.

The current account, budget, and change implementation files are removed.
Public integration tests compare the ownership semantics of the standard and
tracked wrappers. Public lifecycle performance is measured by one Criterion
integration benchmark with matched standard-library baselines.

## Verification

`tracked_arc.rs` and `tracked_box.rs` each contain inline unit tests and ignored
Criterion unit benchmarks. Their fixtures locally implement all required
traits.

Tests cover exact creation, clone, update, and drop delta sequences; shared Arc
allocation release exactly once; deep Box clone independence; closure-result
preservation; and wrappers that remain one machine pointer wide.

Each wrapper source file retains one unit benchmark for an important isolated
path: Arc clone without destruction and Box update without setup or
destruction.

`tests/ownership.rs` verifies that tracked Arc sharing and tracked Box deep
cloning preserve their standard-library ownership semantics while accounting
returns to zero after every value is dropped.

`benches/ownership.rs` benchmarks complete public-API lifecycles in batches of
1,000. Every tracked path has a matched standard-library baseline performing
the same ownership and payload work. It compares no-op, local-counter, and
shared-atomic budget strategies and measures deep Box cloning across multiple
retained payload sizes. The README reports the standard median, tracked median,
their derived difference, and throughput. These integration results, rather
than the diagnostic unit measurements, form the performance story.

## Scope exclusions

- No atomic budget implementation.
- No memory-reporting thread.
- No admission or rejection policy.
- No error event type.
- No runtime or core dependency.
- No normal dependency on another ConTime crate.
- No Git commit during this cleanup pass.
