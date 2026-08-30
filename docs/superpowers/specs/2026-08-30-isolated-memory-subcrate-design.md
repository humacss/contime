# Isolated Memory Subcrate Design

Date: 2026-08-30
Status: Approved in conversation; pending written-spec review

## Purpose

Create an isolated `contime-memory` crate that owns ConTime's retained-memory
accounting primitives. The first consumer will be the new apply-only core
pipeline, but this crate must not depend on the root `contime` crate or any
other ConTime subcrate.

The crate will introduce a fallibly cloned, memory-accounted Arc replacement.
It accounts for the shared allocation exactly once and for every live strong
pointer individually. Rust ownership and `Drop` release both reservations
without a central pointer registry or strong-count inspection.

## Goals

- Centralize memory accounting instead of dividing it among the API, router,
  worker, event-history, and checkpoint crates.
- Preserve shared event allocations across pipeline stages.
- Account once for the complete conservative allocation.
- Account once for every live pointer to that allocation.
- Make pointer creation fallible when its memory budget is exhausted.
- Release pointer bytes whenever a pointer is dropped.
- Release allocation bytes exactly when the final pointer is dropped.
- Keep the tracked pointer one machine pointer wide.
- Use atomic, lock-free accounting suitable for router and worker threads.
- Benchmark allocation, cloning, non-final drop, and final drop independently.

## Non-goals

- Horizon advancement or history pruning.
- Choosing which retained event or checkpoint to evict.
- Forcing external pointers to be dropped.
- Transactional admission across several independent memory budgets.
- Measuring exact process RSS or allocator-private metadata.
- Connecting the new type to existing ConTime subcrates in this pass.
- Modifying the root `contime` crate.

## Isolation

`contime-memory` has no normal dependencies beyond the Rust standard library.
It uses `std::sync::Arc` and atomic integers directly. Criterion may be a
development-only dependency for inline unit benchmarks.

The crate declares all of its own traits, errors, categories, accounting
state, and tracked-pointer types. Its tests and benchmarks use only local
fixtures. Other subcrates may later depend on its contracts; dependency arrows
never point back from memory into those crates.

## Public contracts

### Conservative value size

```rust
pub trait ConservativeSize {
    fn conservative_size(&self) -> u64;
}
```

The returned size represents the complete retained value graph owned by the
value. It must include dynamically allocated payloads owned exclusively by the
value. The memory crate ensures the result is never treated as smaller than
`size_of::<T>()` before adding tracked-allocation overhead.

### Accounting categories

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    Allocation,
    Pointer,
}
```

`Allocation` is the shared allocation retained once. `Pointer` is one live
`TrackedArc` handle. Keeping the categories distinct allows the owner to
inspect retained payload and pointer fan-out separately while enforcing one
combined limit.

### Accounting implementation boundary

```rust
pub trait MemoryAccount: Clone + Send + Sync {
    type Error;

    fn try_reserve(
        &self,
        kind: MemoryKind,
        bytes: u64,
    ) -> Result<(), Self::Error>;

    fn release(&self, kind: MemoryKind, bytes: u64);
}
```

Implementations must be safe to call concurrently. A successful reservation
must remain charged until exactly one matching release. A failed reservation
must change no accounting state. `release` is infallible because it only
reconciles a reservation the crate already owns.

### Atomic budget

```rust
#[derive(Clone)]
pub struct MemoryBudget { /* shared atomic state */ }

impl MemoryBudget {
    pub fn new(limit: u64) -> Self;
    pub fn limit(&self) -> u64;
    pub fn used(&self) -> u64;
    pub fn remaining(&self) -> u64;
    pub fn allocation_bytes(&self) -> u64;
    pub fn pointer_bytes(&self) -> u64;
}
```

`MemoryBudget` clones share one state. A compare-and-swap loop reserves against
the combined used-byte limit. Category counters track allocation and pointer
bytes for inspection. Category and total counters are separate atomics, so a
concurrent observer may see a transient cross-counter mismatch while an
operation is committing; after the operation completes, their values agree.

Reservation failure returns a crate-local `MemoryFull` value containing the
requested bytes and a best-effort remaining-byte observation. Arithmetic is
saturating or checked so integer overflow cannot admit memory accidentally.

The `MemoryBudget` handles used internally by tracked allocations are not
recursively accounted as tracked pointers. Their fixed storage is included in
the shared tracked-allocation overhead instead.

## Tracked pointer representation

```rust
pub struct TrackedArc<T, M: MemoryAccount = MemoryBudget> {
    inner: Arc<Allocation<T, M>>,
}

struct Allocation<T, M: MemoryAccount> {
    value: T,
    memory: M,
    allocation_bytes: u64,
}
```

`TrackedArc` contains exactly one `Arc`, so its size is one machine pointer.
The memory-account handle and allocation byte count exist once inside the
shared allocation.

The shared conservative allocation size is calculated from:

1. `max(value.conservative_size(), size_of::<T>())`;
2. the fixed fields stored beside `T` in `Allocation<T, M>`, including
   alignment padding; and
3. a conservative allowance for Arc's strong and weak counters.

The formula models logical retained memory and documented Arc control state.
Allocator-private bookkeeping and process RSS remain outside the contract.
Every addition is saturating; a saturated size will fail any smaller budget.

The pointer reservation is `size_of::<TrackedArc<T, M>>()` and is therefore
one machine pointer on supported targets. A compile-time or unit assertion
protects this representation.

## Construction

```rust
impl<T, M: MemoryAccount> TrackedArc<T, M>
where
    T: ConservativeSize,
{
    pub fn try_new(value: T, memory: M) -> Result<Self, M::Error>;
}
```

Construction follows this order:

1. Calculate allocation and pointer bytes.
2. Reserve `MemoryKind::Allocation`.
3. Reserve `MemoryKind::Pointer`.
4. If pointer reservation fails, release the allocation reservation and return
   the pointer error.
5. Construct the shared `Arc<Allocation<T, M>>` and return its first pointer.

No tracked value exists until both reservations succeed. If allocation of the
underlying Arc aborts, normal Rust allocation-failure behavior applies.

## Fallible cloning

```rust
impl<T, M: MemoryAccount> TrackedArc<T, M> {
    pub fn try_clone(&self) -> Result<Self, M::Error>;
}
```

`try_clone` reserves one pointer before calling `Arc::clone`. A failed
reservation leaves the Arc count and accounting unchanged. A successful clone
adds no allocation charge and preserves the original value address.

`TrackedArc` deliberately does not implement `Clone`. Every new strong pointer
must pass through the fallible accounting operation. It implements `Deref`,
`AsRef`, and suitable debug/equality forwarding without exposing the inner
standard Arc.

## Drop behavior

Every `TrackedArc::drop` releases exactly one pointer reservation. Rust then
drops its inner Arc field normally.

`Allocation::drop` releases the allocation reservation stored in
`allocation_bytes`. Rust invokes that destructor exactly once, when the final
strong Arc pointer disappears. This avoids `Arc::strong_count`, pointer
registries, callbacks from consumers, and race-prone last-pointer checks.

The final pointer drop therefore performs two releases in this order:

1. release its pointer bytes;
2. release the shared allocation bytes when Arc destroys `Allocation`.

## Source layout

- `lib.rs`: module declarations and narrow public re-exports only.
- `types.rs`: public traits, enums, errors, and struct definitions.
- `budget.rs`: atomic `MemoryBudget` accounting and inspection.
- `new.rs`: conservative allocation sizing and `try_new`.
- `clone.rs`: `try_clone`.
- `drop.rs`: pointer and allocation destruction.
- `access.rs`: `Deref`, `AsRef`, and non-owning convenience behavior.

Tests and ignored inline Criterion benchmarks live beside the operation they
exercise. There is no integration-test directory in the first pass.

## Unit tests

Tests must prove:

- a first pointer charges one allocation and one pointer;
- a successful clone charges only one pointer;
- a failed clone does not alter the strong count or accounting;
- dropping a non-final pointer releases only one pointer;
- dropping the final pointer also releases the allocation;
- failed pointer reservation during construction rolls back allocation bytes;
- failed allocation reservation performs no pointer reservation;
- all accounting returns to zero after concurrent clone/drop activity;
- all clones dereference to the same value address;
- the wrapper remains one pointer wide;
- zero-sized and dynamically conservative values are accounted
  conservatively;
- overflow cannot bypass the limit.

Tests synchronize using thread joins and exact atomic state. They use no sleeps
or polling.

## Unit benchmarks

Ignored inline Criterion benchmarks measure one public unit at a time:

- `MemoryBudget::try_reserve` followed by matching release;
- `TrackedArc::try_new` with teardown outside the measured construction where
  Criterion permits;
- `TrackedArc::try_clone` with prepared source allocation;
- non-final pointer drop;
- final pointer drop.

Fixtures, budget creation, and unrelated teardown stay outside the timed
operation. Each benchmark has a corresponding correctness test. Results and
exact commands are recorded in the crate README after implementation.

## Future integration

After this crate is validated, the core intermediary types will use
`TrackedArc` rather than standard `Arc`. The API, router, worker, event, and
checkpoint crates will expose their local input/output traits, and core-owned
types will implement both sides of each boundary.

Router fan-out will use `try_clone`, making additional snapshot routes subject
to memory admission. Event histories retain tracked pointers without owning
separate payload accounting. Checkpoint memory can use the same
`MemoryAccount` contract even when it does not need shared pointers.

Horizon advancement will later release memory naturally by dropping retained
tracked pointers and checkpoints. No advancement behavior is part of this
crate's initial implementation.
