# Isolated Memory Ownership Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite `contime-memory` around conservative tracked-size measurement, pluggable per-item accounts, infallible atomic aggregate budgeting, and automatically accounted Arc and Box ownership.

**Architecture:** Underlying values implement `ConservativeTrackedSize`; `MeasuredAccount` or `CachedAccount` turns one mutable closure into a `MemoryChange`; one shared `AtomicMemoryBudget` aggregates actual usage without rejecting completed work. `TrackedArc` and `TrackedBox` keep the account and budget inside one heap allocation so each public wrapper remains one machine pointer wide and performs all reserve, resize, clone, and drop accounting automatically.

**Tech Stack:** Rust 2021, standard-library `Arc`, `Box`, channels, and atomics, Criterion 0.5 as a development-only dependency.

**Spec:** `crates/memory/docs/design.md`

## Global Constraints

- Modify, stage, and commit only `crates/memory`.
- Keep the crate isolated from root ConTime and every sibling ConTime subcrate.
- Use `usize` for sizes and `AtomicUsize` for shared counters.
- Use no mutexes and no fallible runtime reserve/resize/release operations.
- Keep `TrackedArc` and `TrackedBox` exactly one machine pointer wide.
- Do not implement `ConservativeTrackedSize` for either tracked wrapper.
- Do not implement `DerefMut` or `into_inner` for `TrackedBox`.
- Make `MeasuredAccount` the default and `CachedAccount` opt-in.
- Keep shared types in `types.rs`; keep `lib.rs` limited to modules and re-exports.
- Put unit tests and ignored Criterion unit benchmarks inline with their behavior-owning source files.
- Use integration tests for complete ownership behavior and a real Criterion integration benchmark for 1,000-operation flows.
- Run all Cargo commands with `CARGO_TARGET_DIR=/private/tmp/contime-memory-target`.

## File Structure

- Rewrite `crates/memory/src/lib.rs`: module declarations and public re-exports only.
- Rewrite `crates/memory/src/types.rs`: all public traits, enums, configuration/state types, concrete account/budget/wrapper type declarations, and private allocation state.
- Create `crates/memory/src/change.rs`: `MemoryChange::between`.
- Create `crates/memory/src/measured_account.rs`: zero-sized measure-twice account.
- Create `crates/memory/src/cached_account.rs`: opt-in cached account.
- Rewrite `crates/memory/src/budget.rs`: configuration validation and lock-free aggregate accounting.
- Create `crates/memory/src/tracked_arc.rs`: complete shared ownership lifecycle.
- Create `crates/memory/src/tracked_box.rs`: complete exclusive ownership and mutation lifecycle.
- Delete `crates/memory/src/access.rs`, `clone.rs`, `drop.rs`, and `new.rs` after their behavior is replaced.
- Create `crates/memory/tests/lifecycle.rs`: public cross-unit ownership tests.
- Create `crates/memory/benches/lifecycle.rs`: 1,000-message and 1,000-snapshot flows.
- Modify `crates/memory/Cargo.toml`: register the integration benchmark.
- Rewrite `crates/memory/README.md`: contracts, commands, benchmark boundaries, and measured results.

---

### Task 1: Shared Vocabulary and Memory Changes

**Files:**
- Rewrite: `crates/memory/src/lib.rs`
- Rewrite: `crates/memory/src/types.rs`
- Create: `crates/memory/src/change.rs`

**Interfaces:**
- Consumes: Rust `usize` and `FnOnce` vocabulary.
- Produces: `ConservativeTrackedSize`, `MemoryChange`, `MemoryAccount<T>`, `MemoryBudget`, `MemoryKind`, `MemoryStatus`, `MemoryState`, `MemoryBudgetConfig`, and `MemoryBudgetConfigError`.

- [ ] **Step 1: Replace the public shell and write failing change tests**

Make `lib.rs` expose only the new vocabulary initially:

```rust
//! Isolated ownership-driven memory accounting.

mod change;
mod types;

pub use types::{
    ConservativeTrackedSize, MemoryAccount, MemoryBudget, MemoryBudgetConfig,
    MemoryBudgetConfigError, MemoryChange, MemoryKind, MemoryState,
    MemoryStatus,
};
```

Define the public vocabulary in `types.rs`:

```rust
pub trait ConservativeTrackedSize {
    fn conservative_tracked_size(&self) -> usize;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryChange {
    Increase(usize),
    Decrease(usize),
    Unchanged,
}

pub trait MemoryAccount<T>: Sized
where
    T: ConservativeTrackedSize,
{
    fn new(value: &T) -> Self;
    fn current(&self, value: &T) -> usize;
    fn change<R, F>(&mut self, value: &mut T, action: F) -> (R, MemoryChange)
    where
        F: FnOnce(&mut T) -> R;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind { Allocation, Pointer }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryStatus { Ready, ActionBlocked, HardLimitExceeded }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryState {
    pub used: usize,
    pub allocation_bytes: usize,
    pub pointer_bytes: usize,
    pub action_ceiling: usize,
    pub hard_limit: usize,
    pub status: MemoryStatus,
    pub buffer_exceeded_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryBudgetConfig {
    pub hard_limit: usize,
    pub concurrent_actions: usize,
    pub action_buffer: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryBudgetConfigError {
    HeadroomOverflow,
    HeadroomExceedsHardLimit,
}

pub trait MemoryBudget: Clone + Send + Sync {
    fn reserve(&self, kind: MemoryKind, bytes: usize);
    fn resize(&self, kind: MemoryKind, change: MemoryChange);
    fn release(&self, kind: MemoryKind, bytes: usize);
    fn state(&self) -> MemoryState;
}
```

Later tasks add the concrete account, budget, and wrapper structs to `types.rs`. In `change.rs`, add tests for increase, decrease, equality, zero, and the complete `usize` range before implementing `between`.

- [ ] **Step 2: Run the focused test and verify the red failure**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib change::tests
```

Expected: compilation fails because `MemoryChange::between` is missing.

- [ ] **Step 3: Implement the change calculation and inline benchmark**

Implement:

```rust
impl MemoryChange {
    pub fn between(before: usize, after: usize) -> Self {
        match after.cmp(&before) {
            std::cmp::Ordering::Greater => Self::Increase(after - before),
            std::cmp::Ordering::Less => Self::Decrease(before - after),
            std::cmp::Ordering::Equal => Self::Unchanged,
        }
    }
}
```

Add ignored `benchmark_change` measuring 1,000 `between` calls with mixed increase/decrease/unchanged inputs.

- [ ] **Step 4: Verify the vocabulary unit**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib
```

Expected: all new change tests pass and the benchmark is ignored.

- [ ] **Step 5: Commit only this unit**

```bash
git add crates/memory/src/lib.rs crates/memory/src/types.rs crates/memory/src/change.rs
git commit -m "refactor(memory): define ownership accounting vocabulary"
```

---

### Task 2: Measured and Cached Accounts

**Files:**
- Modify: `crates/memory/src/lib.rs`
- Modify: `crates/memory/src/types.rs`
- Create: `crates/memory/src/measured_account.rs`
- Create: `crates/memory/src/cached_account.rs`

**Interfaces:**
- Consumes: `ConservativeTrackedSize`, `MemoryAccount<T>`, and `MemoryChange::between`.
- Produces: zero-sized `MeasuredAccount` and one-`usize` `CachedAccount`.

- [ ] **Step 1: Declare account types and write failing measurement-count tests**

Add to `types.rs`:

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MeasuredAccount;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CachedAccount {
    pub(crate) bytes: usize,
}
```

Add modules and re-exports in `lib.rs`. In each account file, use a fixture whose `conservative_tracked_size` increments a `Cell<usize>`. Assert:

- measured construction performs no measurement;
- measured `current` performs one measurement;
- measured `change` performs exactly two measurements;
- cached construction performs one measurement;
- cached `current` performs no additional measurement;
- cached `change` performs exactly one additional measurement and updates the cached value;
- both accounts return the closure result and correct increase/decrease/unchanged change.

- [ ] **Step 2: Run the account tests and verify the red failure**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib account
```

Expected: compilation fails because neither `MemoryAccount` implementation exists.

- [ ] **Step 3: Implement both account strategies**

Measured behavior:

```rust
impl<T> MemoryAccount<T> for MeasuredAccount
where
    T: ConservativeTrackedSize,
{
    fn new(_value: &T) -> Self { Self }
    fn current(&self, value: &T) -> usize { value.conservative_tracked_size() }

    fn change<R, F>(&mut self, value: &mut T, action: F) -> (R, MemoryChange)
    where F: FnOnce(&mut T) -> R,
    {
        let before = value.conservative_tracked_size();
        let result = action(value);
        let after = value.conservative_tracked_size();
        (result, MemoryChange::between(before, after))
    }
}
```

Cached behavior uses `self.bytes` as `before`, runs the action, measures once, updates `self.bytes`, and returns `MemoryChange::between(before, after)`.

- [ ] **Step 4: Add isolated account benchmarks**

In each file add an ignored Criterion benchmark with two fixtures:

- cheap fixed-layout sizing;
- expensive sizing that walks 1,000 stored `usize` values.

Benchmark 1,000 account changes while reusing the same account and value. Keep fixture construction outside the timed closure.

- [ ] **Step 5: Verify account behavior and layout**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib
```

Expected: all tests pass; assert `size_of::<MeasuredAccount>() == 0` and `size_of::<CachedAccount>() == size_of::<usize>()`.

- [ ] **Step 6: Commit only the account units**

```bash
git add crates/memory/src/lib.rs crates/memory/src/types.rs \
  crates/memory/src/measured_account.rs crates/memory/src/cached_account.rs
git commit -m "feat(memory): add measured and cached accounts"
```

---

### Task 3: Lock-Free Aggregate Budget

**Files:**
- Modify: `crates/memory/src/lib.rs`
- Modify: `crates/memory/src/types.rs`
- Rewrite: `crates/memory/src/budget.rs`

**Interfaces:**
- Consumes: `MemoryBudget`, `MemoryBudgetConfig`, `MemoryChange`, `MemoryKind`, `MemoryState`, and `MemoryStatus`.
- Produces: `AtomicMemoryBudget::new(config) -> Result<Self, MemoryBudgetConfigError>` and atomic reserve/resize/release/state behavior.

- [ ] **Step 1: Declare atomic state and write failing configuration tests**

Add to `types.rs`:

```rust
#[derive(Clone)]
pub struct AtomicMemoryBudget {
    pub(crate) state: Arc<AtomicMemoryState>,
}

pub(crate) struct AtomicMemoryState {
    pub(crate) hard_limit: usize,
    pub(crate) action_ceiling: usize,
    pub(crate) action_buffer: usize,
    pub(crate) used: AtomicUsize,
    pub(crate) allocation_bytes: AtomicUsize,
    pub(crate) pointer_bytes: AtomicUsize,
    pub(crate) buffer_exceeded_count: AtomicUsize,
}
```

Write tests first for zero workers, checked multiplication overflow, headroom exceeding the hard limit, and an exact ceiling calculation.

- [ ] **Step 2: Run configuration tests and verify red**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib budget::tests::configuration
```

Expected: compilation fails because `AtomicMemoryBudget::new` is missing.

- [ ] **Step 3: Implement checked construction and state classification**

Use `checked_mul` and `checked_sub`. Classify with:

```rust
fn status(used: usize, action_ceiling: usize, hard_limit: usize) -> MemoryStatus {
    if used > hard_limit {
        MemoryStatus::HardLimitExceeded
    } else if used > action_ceiling {
        MemoryStatus::ActionBlocked
    } else {
        MemoryStatus::Ready
    }
}
```

`MemoryState` is a non-transactional observation of independent atomic counters; tests inspect it after worker threads join.

- [ ] **Step 4: Write failing reserve/resize/release and concurrency tests**

Cover:

- allocation and pointer categories sum into total usage;
- increase/decrease/unchanged resize behavior;
- crossing and recovering below the action ceiling;
- crossing the hard limit without rejecting the completed operation;
- an increase greater than `action_buffer` increments `buffer_exceeded_count`;
- 8 threads each reserve and release 10,000 pointer bytes and return all counters to zero;
- aggregate addition never wraps at `usize::MAX`;
- release underflow panics.

- [ ] **Step 5: Implement atomic accounting**

Use compare-and-update loops for saturating addition and checked subtraction. Update total and category counters with `Ordering::AcqRel`; observe with `Ordering::Acquire`. `resize(Increase)` delegates to reserve and records a buffer violation when applicable; `resize(Decrease)` delegates to release; `Unchanged` performs no counter mutation.

- [ ] **Step 6: Add unit benchmarks for each operation**

Add ignored Criterion measurements for 1,000 allocation reserves, 1,000 pointer reserves, 1,000 increases, 1,000 decreases, and 1,000 balanced reserve/releases. Reuse one budget and reset outside timed regions where the named operation requires it.

- [ ] **Step 7: Verify the complete budget unit**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib budget
```

Expected: all configuration, threshold, category, concurrency, overflow, and underflow tests pass.

- [ ] **Step 8: Commit only the budget unit**

```bash
git add crates/memory/src/lib.rs crates/memory/src/types.rs crates/memory/src/budget.rs
git commit -m "feat(memory): add lock-free aggregate budget"
```

---

### Task 4: Tracked Arc Lifecycle

**Files:**
- Modify: `crates/memory/src/lib.rs`
- Modify: `crates/memory/src/types.rs`
- Create: `crates/memory/src/tracked_arc.rs`

**Interfaces:**
- Consumes: `ConservativeTrackedSize`, `MemoryAccount<T>`, `MemoryBudget`, `MemoryKind`, and `MeasuredAccount`.
- Produces: `TrackedArc<T, A = MeasuredAccount, B = AtomicMemoryBudget>` with `new`, ordinary `Clone`, immutable access, pointer accounting, and final allocation release.

- [ ] **Step 1: Declare the one-pointer wrapper and write failing lifecycle tests**

Add these type declarations to `types.rs`:

```rust
pub struct TrackedArc<T, A = MeasuredAccount, B = AtomicMemoryBudget>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    pub(crate) inner: Arc<ArcAllocation<T, A, B>>,
}

pub(crate) struct ArcAllocation<T, A, B>
where
    T: ConservativeTrackedSize,
    A: MemoryAccount<T>,
    B: MemoryBudget,
{
    pub(crate) value: T,
    pub(crate) account: A,
    pub(crate) budget: B,
}
```

Write tests for initial allocation/pointer charges, standard clone, non-final drop, final drop, immutable dereference, debug/equality forwarding, 32 concurrent clone drops, and one-pointer layout.

- [ ] **Step 2: Run Arc tests and verify red**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib tracked_arc
```

Expected: compilation fails because `TrackedArc::new` and lifecycle implementations are missing.

- [ ] **Step 3: Implement conservative allocation composition**

Calculate shared allocation bytes as:

```text
account.current(value)
+ (size_of::<ArcAllocation<T, A, B>>() - size_of::<T>())
+ 2 * size_of::<AtomicUsize>()
```

The tracked-size contract must include all non-tracked inline and retained memory belonging to `T`; it deliberately excludes nested tracked wrappers because those wrappers account for themselves. Do not reintroduce the full `size_of::<T>()` as a lower bound, because that would double-count nested tracked handles. Use checked additions that resolve to `usize::MAX`. The two atomics represent Arc strong/weak counters. Pointer bytes are `size_of::<Self>()`.

- [ ] **Step 4: Implement new, Clone, Drop, and immutable forwarding**

`new` creates `A`, reserves allocation and first pointer, then creates the Arc. `Clone` reserves one pointer before `Arc::clone`. Wrapper `Drop` releases one pointer; `ArcAllocation::drop` measures and releases the final allocation. Add `Deref`, `AsRef`, `Debug`, `PartialEq`, and `Eq`; do not add mutable access or `ConservativeTrackedSize`.

- [ ] **Step 5: Add isolated Arc benchmarks**

Add ignored Criterion benchmarks for new, clone, non-final drop, and final drop. Prepare the original Arc outside clone/drop timing and use batched setup so fixture creation is excluded unless `new` is named.

- [ ] **Step 6: Verify the Arc unit**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib tracked_arc
```

Expected: all Arc tests pass and `size_of::<TrackedArc<Fixture>>() == size_of::<usize>()`.

- [ ] **Step 7: Commit only the Arc unit**

```bash
git add crates/memory/src/lib.rs crates/memory/src/types.rs crates/memory/src/tracked_arc.rs
git commit -m "feat(memory): add automatically accounted Arc"
```

---

### Task 5: Tracked Box Lifecycle and Measured Mutation

**Files:**
- Modify: `crates/memory/src/lib.rs`
- Modify: `crates/memory/src/types.rs`
- Create: `crates/memory/src/tracked_box.rs`
- Delete: `crates/memory/src/access.rs`
- Delete: `crates/memory/src/clone.rs`
- Delete: `crates/memory/src/drop.rs`
- Delete: `crates/memory/src/new.rs`

**Interfaces:**
- Consumes: the same measurement/account/budget traits as `TrackedArc`.
- Produces: `TrackedBox<T, A = MeasuredAccount, B = AtomicMemoryBudget>` with `new`, deep `Clone`, immutable access, `update`, and automatic drop accounting.

- [ ] **Step 1: Declare the one-pointer Box and write failing lifecycle tests**

Add `TrackedBox` and private `BoxAllocation` declarations mirroring the Arc declarations but using `Box<BoxAllocation<T, A, B>>`. Write tests for initial charges, deep independent clone, clone account independence, pointer/allocation drop, immutable forwarding, one-pointer layout, update closure result, and grow/shrink/unchanged updates under both account strategies.

- [ ] **Step 2: Run Box tests and verify red**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib tracked_box
```

Expected: compilation fails because the Box lifecycle is missing.

- [ ] **Step 3: Implement Box allocation composition and ownership**

Calculate allocation bytes as:

```text
account.current(value)
+ (size_of::<BoxAllocation<T, A, B>>() - size_of::<T>())
```

As with `TrackedArc`, trust the tracked-size contract instead of applying a `size_of::<T>()` lower bound that would double-count nested tracked handles. Use checked additions resolving to `usize::MAX`. Reserve one outer pointer separately. `Clone` must call `T::clone`, create a fresh `A`, and reserve a new allocation and pointer; it must not clone the source account.

- [ ] **Step 4: Implement measured update**

Implement:

```rust
pub fn update<R>(&mut self, action: impl FnOnce(&mut T) -> R) -> R {
    let (result, change) = self.inner.account.change(&mut self.inner.value, action);
    self.inner.budget.resize(MemoryKind::Allocation, change);
    result
}
```

Provide immutable forwarding only. Wrapper drop releases its pointer; inner allocation drop releases `allocation_bytes(current)`.

- [ ] **Step 5: Add isolated Box benchmarks**

Add ignored Criterion benchmarks for new, deep clone, measured update, cached update, ordinary drop, and a 1,000-element vector growth action. Keep deep-clone source and update fixtures outside timed regions where appropriate.

- [ ] **Step 6: Remove superseded implementation files and verify the crate**

Delete the four old lifecycle files only after both wrappers replace them. Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib
```

Expected: all unit tests pass; ignored unit benchmarks compile.

- [ ] **Step 7: Commit only the Box rewrite and cleanup**

```bash
git add crates/memory/src
git commit -m "feat(memory): add measured tracked Box ownership"
```

---

### Task 6: Public Ownership Integration Tests

**Files:**
- Create: `crates/memory/tests/lifecycle.rs`

**Interfaces:**
- Consumes: only the public exports of `contime-memory` and `std::sync::mpsc`.
- Produces: cross-unit evidence for message, event, snapshot, aggregate, and action-headroom behavior.

- [ ] **Step 1: Write the complete channel-message test**

Define an event implementing `ConservativeTrackedSize`, wrap it in `TrackedArc`, place a clone inside a message whose tracked-size implementation excludes the nested tracked wrapper, wrap the message in `TrackedBox`, and move it through `std::sync::mpsc::channel`. Assert state is unchanged by send/receive and returns to zero only after the received message and remaining event pointer are dropped.

- [ ] **Step 2: Write snapshot and aggregate tests**

Add tests proving:

- a deeply cloned tracked snapshot mutates independently;
- measured and cached snapshot updates produce identical aggregate usage;
- multiple Arc, Box, pointer, and message values aggregate into one budget;
- a completed update may move state to `ActionBlocked` without losing its value;
- releases move the budget back to `Ready`;
- a single resize larger than the configured buffer increments the diagnostic count while retaining the value.

- [ ] **Step 3: Run integration tests**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --test lifecycle
```

Expected: every public lifecycle test passes.

- [ ] **Step 4: Commit only integration tests**

```bash
git add crates/memory/tests/lifecycle.rs
git commit -m "test(memory): cover complete ownership lifecycles"
```

---

### Task 7: Integration Benchmarks

**Files:**
- Modify: `crates/memory/Cargo.toml`
- Create: `crates/memory/benches/lifecycle.rs`

**Interfaces:**
- Consumes: public tracked wrappers, both account strategies, and atomic budget.
- Produces: Criterion groups `memory/lifecycle/messages` and `memory/lifecycle/snapshots`.

- [ ] **Step 1: Register the benchmark target**

Add:

```toml
[[bench]]
name = "lifecycle"
harness = false
```

- [ ] **Step 2: Implement the 1,000-message benchmark**

Prepare 1,000 tracked event pointers and message payload data outside timing. Inside timing, create 1,000 tracked message boxes, move them through a pre-created standard channel, receive them, and drop them. Report total time and Criterion throughput of 1,000 elements. Do not include budget construction or source-event construction.

- [ ] **Step 3: Implement measured and cached 1,000-update benchmarks**

Prepare one tracked snapshot holding a preallocated vector. In each timed iteration perform 1,000 fixed-size mutations through `TrackedBox::update`; benchmark `MeasuredAccount` and `CachedAccount` separately with identical snapshot and action code. Reset state in batched setup, outside the timed closure.

- [ ] **Step 4: Compile and run the integration benchmarks**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo bench --manifest-path crates/memory/Cargo.toml --bench lifecycle
```

Expected: three lifecycle measurements complete without accounting invariant failures, and every iteration returns its budget to the expected final state.

- [ ] **Step 5: Commit only benchmark files**

```bash
git add crates/memory/Cargo.toml crates/memory/benches/lifecycle.rs
git commit -m "bench(memory): measure ownership lifecycle throughput"
```

---

### Task 8: Documentation, Full Benchmarks, and Isolation Audit

**Files:**
- Rewrite: `crates/memory/README.md`
- Modify only if verification exposes a defect: files under `crates/memory/src`, `tests`, or `benches`.

**Interfaces:**
- Consumes: all completed units and measured Criterion output.
- Produces: documented API contracts, exact benchmark boundaries/results, and a verified isolated crate.

- [ ] **Step 1: Run every ignored unit benchmark individually**

Use release-mode filtered commands so each source unit has a readable result:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target cargo test \
  --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_change -- --ignored --nocapture --test-threads=1
```

Repeat for measured account, cached account, budget, tracked Arc, and tracked Box benchmark filters. Record medians without combining fixture setup into the named operation.

- [ ] **Step 2: Rewrite the README**

Document:

- the three-trait model;
- conservative current-size semantics;
- measured default and cached opt-in trade-off;
- infallible accounting and action-buffer policy;
- Arc/Box clone and drop behavior;
- message ownership behavior;
- unit and integration commands;
- a benchmark table containing operation, batch size, median, per-operation cost, and throughput where meaningful;
- explicit exclusions for every benchmark.

- [ ] **Step 3: Run final formatting, tests, lint, dependency, and diff checks**

Run:

```bash
cargo fmt --manifest-path crates/memory/Cargo.toml -- --check
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo clippy --manifest-path crates/memory/Cargo.toml --all-targets -- -D warnings
cargo tree --manifest-path crates/memory/Cargo.toml -e normal
git diff --check -- crates/memory
```

Expected: formatting succeeds; all unit and integration tests pass; strict Clippy succeeds; the normal dependency tree contains only `contime-memory`; and the path-scoped diff check is empty.

- [ ] **Step 4: Audit the public API against the specification**

Confirm:

- `lib.rs` contains only modules and re-exports;
- all shared types are in `types.rs`;
- no old `ConservativeSize`, `MemoryFull`, `try_reserve`, `try_clone`, or retained-delta API remains;
- no wrapper implements `ConservativeTrackedSize`;
- no Box mutable dereference or extraction API exists;
- both wrappers are one pointer wide;
- all non-doc changes are under `crates/memory`.

- [ ] **Step 5: Commit only final memory documentation and verified fixes**

```bash
git add crates/memory
git diff --cached --name-only
git commit -m "docs(memory): document ownership accounting benchmarks"
```

Before committing, the staged path list must contain only `crates/memory`.
