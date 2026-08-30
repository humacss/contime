# Isolated Memory Ownership Design

## Purpose

`contime-memory` is the only subsystem that performs memory accounting. Values
describe their current size, per-item accounts measure mutations, tracked
wrappers bind accounting to ownership, and a shared budget aggregates the
result. API, router, worker, events, checkpoints, lanes, runtime, and core must
not manually exchange retained-memory deltas after adopting this contract.

This pass rewrites only the isolated memory crate. Core integration and removal
of old downstream accounting are separate passes.

## Design rules

- Use `usize` for all in-process memory sizes.
- Use no mutexes. The concrete shared budget uses `AtomicUsize`.
- Runtime accounting records reality and is infallible. It never rolls back or
  drops a completed action because a threshold was crossed.
- A configured action buffer protects allocations made between the before and
  after measurements of synchronous worker actions.
- Crossing the action ceiling blocks future memory-growing actions; it does not
  invalidate the action that just completed.
- A buffer violation is an operational diagnostic, not an event rejection.
- Wrapper `Drop` implementations release memory automatically.
- Standard `Clone` is supported: Arc cloning accounts another pointer, while
  Box cloning deeply clones and separately accounts its pointee.
- The tracked wrappers remain exactly one machine pointer wide.
- `TrackedBox` exposes no unrestricted mutable dereference. Mutation occurs
  only through its measured action closure.
- No tracked wrapper implements `ConservativeTrackedSize`; that trait is for
  the underlying `T` only.

## Shared vocabulary

### Conservative tracked size

```rust
pub trait ConservativeTrackedSize {
    fn conservative_tracked_size(&self) -> usize;
}
```

The method returns the largest reasonable number of bytes currently
attributable to the underlying value `T`. It does not predict unbounded future
growth. Implementations use collection capacity rather than length and choose
the larger current estimate when allocator or ownership details are ambiguous.
Arithmetic that cannot be represented resolves conservatively to `usize::MAX`.

The memory crate adds the outer tracked pointer, account, budget handle, Arc
control block, and other wrapper machinery. Consumers do not include that
machinery in `T::conservative_tracked_size`. Ordinary pointers owned beneath
`T` may be included conservatively. Nested `TrackedArc` and `TrackedBox`
handles are excluded because those wrappers already account their own pointers
and pointees.

### Memory changes

```rust
pub enum MemoryChange {
    Increase(usize),
    Decrease(usize),
    Unchanged,
}
```

`MemoryChange::between(before, after)` compares two `usize` measurements
without signed conversion.

### Per-item measurement account

```rust
pub trait MemoryAccount<T>: Sized
where
    T: ConservativeTrackedSize,
{
    fn new(value: &T) -> Self;
    fn current(&self, value: &T) -> usize;

    fn change<R, F>(
        &mut self,
        value: &mut T,
        action: F,
    ) -> (R, MemoryChange)
    where
        F: FnOnce(&mut T) -> R;
}
```

The closure is the complete mutable action boundary. Its domain result is
returned alongside the internally consumed memory change.

The crate supplies two strategies:

- `MeasuredAccount` is the default and should be zero-sized. It measures before
  and after each mutation and measures the current value for reserve/release.
- `CachedAccount` stores one `usize`. It measures once at creation, uses that
  cached value as the before-size, measures once afterward, and updates its
  cache. Its additional storage is included in wrapper overhead.

Caching is opt-in. Most values should use `MeasuredAccount`; caching is useful
only when measurement cost justifies its persistent storage and consistency
cost.

A panic inside the action is a process-level failure. The hot path does not use
`catch_unwind` merely to preserve accounting after a panic.

## Aggregate budget

```rust
pub trait MemoryBudget: Clone + Send + Sync {
    fn reserve(&self, kind: MemoryKind, bytes: usize);
    fn resize(&self, kind: MemoryKind, change: MemoryChange);
    fn release(&self, kind: MemoryKind, bytes: usize);
    fn state(&self) -> MemoryState;
}
```

`reserve`, `resize`, and `release` increment or decrement aggregate counters.
They do not grant permission and do not return ordinary runtime errors.

Initial categories are:

```rust
pub enum MemoryKind {
    Allocation,
    Pointer,
}
```

The concrete `AtomicMemoryBudget` is configured with:

```rust
pub struct MemoryBudgetConfig {
    pub hard_limit: usize,
    pub concurrent_actions: usize,
    pub action_buffer: usize,
}
```

Construction uses checked arithmetic:

```text
reserved headroom = concurrent_actions * action_buffer
action ceiling    = hard limit - reserved headroom
```

An invalid or overflowing configuration is rejected during construction.
There is no physical allocation for the buffer; it is accounting headroom.

`MemoryState` reports total, allocation, and pointer usage; action ceiling;
hard limit; whether another action may begin; whether the hard limit has been
crossed; and how many resize increases exceeded one configured action buffer.

The states are:

- `Ready`: current usage is at or below the action ceiling.
- `ActionBlocked`: usage is above the action ceiling but not the hard limit.
- `HardLimitExceeded`: measured usage is above the configured hard limit.

All already-running synchronous actions may complete. Core later checks the
state before beginning another memory-growing action. Reads, drops, and future
horizon advancement remain possible while action growth is blocked.

Counter overflow never wraps. Release underflow is a programming invariant,
not a recoverable runtime condition.

## Tracked Arc

`TrackedArc<T, A, B>` is one machine pointer wide. The shared heap allocation
stores `T`, account strategy `A`, and budget handle `B`.

- `new(value, budget)` creates the account, reserves the complete shared
  allocation, and reserves the first outer pointer.
- `Clone` shares the allocation and reserves one additional pointer.
- Every handle drop releases one pointer.
- Dropping the final handle releases the shared allocation using the account's
  current measurement.
- Only immutable `Deref<Target = T>` is exposed.

The shared charge includes the conservative tracked size of `T` plus account,
budget-handle, Arc control-block, and fixed allocation fields. Pointer charge is
`size_of::<TrackedArc<T, A, B>>()`.

## Tracked Box

`TrackedBox<T, A, B>` is one machine pointer wide. Its heap allocation stores
`T`, account strategy `A`, and budget handle `B`.

- `new(value, budget)` creates and reserves one exclusive allocation and its
  outer pointer.
- `Clone`, when `T: Clone`, deeply clones `T`, creates a fresh account, and
  reserves a completely independent allocation and pointer.
- Immutable `Deref<Target = T>` is exposed.
- `DerefMut` is not implemented.
- `update(action)` delegates mutation to `A::change`, forwards the returned
  `MemoryChange` immediately to the budget, and returns only the action's
  domain result.
- Drop releases the latest measured allocation and the outer pointer.

No initial `into_inner` operation is exposed because it would return live,
untracked memory after releasing its accounting.

Channel messages use `TrackedBox`; a separate tracked-message type is
unnecessary. Moving a tracked message through queues changes no accounting.
Dropping its final owner releases it regardless of which subsystem performed
the drop. Channel implementation capacity is runtime infrastructure overhead
and is not attributed to individual live messages.

## Downstream contract

After later integration:

- owned events are wrapped once and shared as `TrackedArc` values;
- every event pointer clone is accounted automatically;
- snapshots and messages use `TrackedBox`;
- checkpoint/replay apply results no longer contain retained-memory deltas;
- the worker's manual memory counter and delta reconciliation disappear;
- core selects concrete account and budget implementations and observes budget
  state and buffer diagnostics.

Those downstream changes are intentionally outside this rewrite.

## Source units

The crate follows the same organization as the other isolated ConTime crates:

- `lib.rs`: module declarations and public re-exports only.
- `types.rs`: public traits, enums, state, configuration, and errors.
- `change.rs`: `MemoryChange` construction and helpers.
- `measured_account.rs`: default measure-twice account.
- `cached_account.rs`: opt-in cached account.
- `budget.rs`: lock-free atomic aggregate budget.
- `tracked_arc.rs`: complete Arc ownership lifecycle.
- `tracked_box.rs`: complete Box ownership and measured mutation lifecycle.

Every behavior-owning source unit contains focused unit tests and an ignored
inline Criterion unit benchmark. Shared vocabulary and re-export files do not
need benchmarks.

## Verification

Unit tests cover change arithmetic, measurement call counts, account mutation,
budget thresholds and categories, concurrent atomic updates, wrapper layout,
deep versus shared clone behavior, measured mutation, and every drop path.

Integration tests cover complete channel-message, shared-event, mutable
snapshot, aggregate-budget, and action-blocking lifecycles.

Unit benchmarks isolate:

- reserve, resize, and release;
- cheap and expensive measured versus cached account changes;
- Arc clone and non-final/final drops;
- Box creation, deep clone, update, and drop.

Integration benchmarks measure 1,000-message ownership lifecycles and
1,000-snapshot-update lifecycles. Fixture construction is outside timed regions
unless construction is the operation named by the benchmark. Measured results
and exact boundaries are recorded in the crate README.

The crate has no normal dependency on root ConTime or another ConTime
subcrate. Criterion remains development-only.
