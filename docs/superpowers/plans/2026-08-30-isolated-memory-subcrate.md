# Isolated Memory Subcrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an isolated `contime-memory` crate with a one-pointer-wide, fallibly cloned tracked Arc and atomic allocation/pointer memory accounting.

**Architecture:** `TrackedArc<T, M>` owns one `Arc<Allocation<T, M>>`. The shared allocation stores the value, one memory-account handle, and its conservative allocation charge; each tracked pointer reserves and releases its own handle bytes, while `Allocation::drop` releases the shared charge exactly once. `MemoryBudget` implements the crate-local `MemoryAccount` trait with lock-free atomic counters and one combined limit.

**Tech Stack:** Rust 2021, Rust standard library `Arc` and atomics, Criterion 0.5 as a development-only dependency.

**Spec:** `docs/superpowers/specs/2026-08-30-isolated-memory-subcrate-design.md`

## Global Constraints

- Create the crate at `crates/memory` with package name `contime-memory`.
- Do not depend on the root `contime` crate or any other ConTime subcrate.
- Do not modify the root crate or any existing subcrate during this plan.
- Preserve all unrelated dirty work; stage and commit only `crates/memory` in implementation commits.
- Use no normal third-party dependencies.
- Keep `TrackedArc<T, M>` exactly one machine pointer wide.
- Do not implement `Clone` for `TrackedArc`; all new pointers use fallible `try_clone`.
- Put unit tests and ignored unit benchmarks inline with their owning source files.
- Do not create an integration-test directory in this pass.
- Use test-first red-green-refactor cycles for each behavior.

## File structure

- Create `crates/memory/Cargo.toml`: isolated package and Criterion development dependency.
- Create `crates/memory/.gitignore`: ignore only crate-local build and Criterion output.
- Create `crates/memory/src/lib.rs`: module declarations and public re-exports.
- Create `crates/memory/src/types.rs`: traits, errors, category enum, budget state, tracked pointer, and shared allocation definitions.
- Create `crates/memory/src/budget.rs`: atomic reservation, release, and inspection.
- Create `crates/memory/src/new.rs`: conservative allocation sizing and `TrackedArc::try_new`.
- Create `crates/memory/src/clone.rs`: `TrackedArc::try_clone`.
- Create `crates/memory/src/drop.rs`: pointer and final-allocation release.
- Create `crates/memory/src/access.rs`: dereference, reference, debug, and equality forwarding.
- Create `crates/memory/README.md`: contracts, verification commands, and measured unit benchmark results.

---

### Task 1: Atomic memory-account contract and budget

**Files:**
- Create: `crates/memory/Cargo.toml`
- Create: `crates/memory/.gitignore`
- Create: `crates/memory/src/lib.rs`
- Create: `crates/memory/src/types.rs`
- Create: `crates/memory/src/budget.rs`

**Interfaces:**
- Consumes: Rust `Arc`, `AtomicU64`, and `Ordering` only.
- Produces: `ConservativeSize`, `MemoryKind`, `MemoryAccount`, `MemoryFull`, and `MemoryBudget`.

- [ ] **Step 1: Create the isolated test shell and failing budget tests**

Create `crates/memory/Cargo.toml`:

```toml
[package]
name = "contime-memory"
version = "0.1.0"
edition = "2021"
autobenches = false
license = "MIT"
description = "Isolated tracked allocation and pointer memory accounting for ConTime"
publish = false

[dependencies]

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
```

Create `crates/memory/.gitignore`:

```gitignore
/target/
/criterion/
```

Create `crates/memory/src/lib.rs` with only module declarations and the intended re-exports:

```rust
//! Isolated retained-allocation and pointer accounting.

mod budget;
mod types;

pub use types::{ConservativeSize, MemoryAccount, MemoryBudget, MemoryFull, MemoryKind};
```

Create `crates/memory/src/budget.rs` with tests that describe the public behavior before those types exist:

```rust
#[cfg(test)]
mod tests {
    use crate::{MemoryAccount, MemoryBudget, MemoryFull, MemoryKind};

    #[test]
    fn reservations_share_one_limit_and_preserve_categories() {
        let memory = MemoryBudget::new(64);

        memory.try_reserve(MemoryKind::Allocation, 40).unwrap();
        memory.try_reserve(MemoryKind::Pointer, 8).unwrap();

        assert_eq!(memory.limit(), 64);
        assert_eq!(memory.used(), 48);
        assert_eq!(memory.remaining(), 16);
        assert_eq!(memory.allocation_bytes(), 40);
        assert_eq!(memory.pointer_bytes(), 8);
    }

    #[test]
    fn failed_reservation_changes_no_accounting_state() {
        let memory = MemoryBudget::new(16);
        memory.try_reserve(MemoryKind::Allocation, 12).unwrap();

        let error = memory.try_reserve(MemoryKind::Pointer, 8).unwrap_err();

        assert_eq!(error, MemoryFull { requested: 8, remaining: 4 });
        assert_eq!(memory.used(), 12);
        assert_eq!(memory.allocation_bytes(), 12);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn release_returns_category_and_total_bytes() {
        let memory = MemoryBudget::new(64);
        memory.try_reserve(MemoryKind::Allocation, 40).unwrap();
        memory.try_reserve(MemoryKind::Pointer, 8).unwrap();

        memory.release(MemoryKind::Pointer, 8);
        memory.release(MemoryKind::Allocation, 40);

        assert_eq!(memory.used(), 0);
        assert_eq!(memory.remaining(), 64);
        assert_eq!(memory.allocation_bytes(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn cloned_budgets_share_state() {
        let memory = MemoryBudget::new(64);
        let clone = memory.clone();

        clone.try_reserve(MemoryKind::Allocation, 24).unwrap();

        assert_eq!(memory.used(), 24);
        memory.release(MemoryKind::Allocation, 24);
        assert_eq!(clone.used(), 0);
    }

    #[test]
    fn overflow_cannot_bypass_the_limit() {
        let memory = MemoryBudget::new(u64::MAX);
        memory.try_reserve(MemoryKind::Allocation, u64::MAX).unwrap();

        assert!(memory.try_reserve(MemoryKind::Pointer, 1).is_err());
        assert_eq!(memory.used(), u64::MAX);
    }
}
```

- [ ] **Step 2: Run the tests and verify the intended red failure**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib
```

Expected: compilation fails because `MemoryAccount`, `MemoryBudget`,
`MemoryFull`, and `MemoryKind` are not yet defined.

- [ ] **Step 3: Implement the public accounting types**

Create `crates/memory/src/types.rs` with these contracts and private state:

```rust
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

pub trait ConservativeSize {
    fn conservative_size(&self) -> u64;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    Allocation,
    Pointer,
}

pub trait MemoryAccount: Clone + Send + Sync {
    type Error;

    fn try_reserve(&self, kind: MemoryKind, bytes: u64) -> Result<(), Self::Error>;
    fn release(&self, kind: MemoryKind, bytes: u64);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryFull {
    pub requested: u64,
    pub remaining: u64,
}

#[derive(Clone)]
pub struct MemoryBudget {
    pub(crate) state: Arc<BudgetState>,
}

pub(crate) struct BudgetState {
    pub(crate) limit: u64,
    pub(crate) used: AtomicU64,
    pub(crate) allocation_bytes: AtomicU64,
    pub(crate) pointer_bytes: AtomicU64,
}
```

- [ ] **Step 4: Implement atomic reservation and release**

Implement `MemoryBudget` and `MemoryAccount` in `crates/memory/src/budget.rs`:

```rust
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::types::{BudgetState, MemoryAccount, MemoryBudget, MemoryFull, MemoryKind};

impl MemoryBudget {
    pub fn new(limit: u64) -> Self {
        Self {
            state: Arc::new(BudgetState {
                limit,
                used: 0.into(),
                allocation_bytes: 0.into(),
                pointer_bytes: 0.into(),
            }),
        }
    }

    pub fn limit(&self) -> u64 {
        self.state.limit
    }

    pub fn used(&self) -> u64 {
        self.state.used.load(Ordering::Acquire)
    }

    pub fn remaining(&self) -> u64 {
        self.limit().saturating_sub(self.used())
    }

    pub fn allocation_bytes(&self) -> u64 {
        self.state.allocation_bytes.load(Ordering::Acquire)
    }

    pub fn pointer_bytes(&self) -> u64 {
        self.state.pointer_bytes.load(Ordering::Acquire)
    }

    fn category(&self, kind: MemoryKind) -> &std::sync::atomic::AtomicU64 {
        match kind {
            MemoryKind::Allocation => &self.state.allocation_bytes,
            MemoryKind::Pointer => &self.state.pointer_bytes,
        }
    }
}

impl MemoryAccount for MemoryBudget {
    type Error = MemoryFull;

    fn try_reserve(&self, kind: MemoryKind, bytes: u64) -> Result<(), Self::Error> {
        let mut current = self.state.used.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(bytes).filter(|next| *next <= self.state.limit) else {
                return Err(MemoryFull {
                    requested: bytes,
                    remaining: self.state.limit.saturating_sub(current),
                });
            };
            match self.state.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.category(kind).fetch_add(bytes, Ordering::Release);
                    return Ok(());
                }
                Err(observed) => current = observed,
            }
        }
    }

    fn release(&self, kind: MemoryKind, bytes: u64) {
        let category_before = self.category(kind).fetch_sub(bytes, Ordering::AcqRel);
        let total_before = self.state.used.fetch_sub(bytes, Ordering::AcqRel);
        assert!(category_before >= bytes, "released more category memory than reserved");
        assert!(total_before >= bytes, "released more total memory than reserved");
    }
}
```

- [ ] **Step 5: Run the focused tests and strict lint**

Run:

```bash
cargo fmt --manifest-path crates/memory/Cargo.toml
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo clippy --manifest-path crates/memory/Cargo.toml --all-targets -- -D warnings
```

Expected: five budget tests pass and Clippy reports no warnings.

- [ ] **Step 6: Commit the atomic budget**

```bash
git add crates/memory
git commit -m "feat(memory): add isolated atomic budget"
```

---

### Task 2: Tracked allocation construction, access, and destruction

**Files:**
- Modify: `crates/memory/src/lib.rs`
- Modify: `crates/memory/src/types.rs`
- Create: `crates/memory/src/new.rs`
- Create: `crates/memory/src/drop.rs`
- Create: `crates/memory/src/access.rs`

**Interfaces:**
- Consumes: `ConservativeSize`, `MemoryAccount`, `MemoryKind`, and `MemoryBudget` from Task 1.
- Produces: `TrackedArc<T, M = MemoryBudget>`, `TrackedArc::try_new`, and read-only value access.

- [ ] **Step 1: Write failing construction and drop tests**

Create `crates/memory/src/new.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use crate::{ConservativeSize, MemoryBudget, TrackedArc};

    #[derive(Debug, Eq, PartialEq)]
    struct Value(u64);

    impl ConservativeSize for Value {
        fn conservative_size(&self) -> u64 {
            64
        }
    }

    #[test]
    fn first_pointer_charges_allocation_and_pointer() {
        let memory = MemoryBudget::new(1_000);
        let value = TrackedArc::try_new(Value(7), memory.clone()).unwrap();

        assert_eq!(*value, Value(7));
        assert_eq!(memory.pointer_bytes(), size_of::<TrackedArc<Value>>() as u64);
        assert!(memory.allocation_bytes() >= 64);
        assert_eq!(memory.used(), memory.allocation_bytes() + memory.pointer_bytes());
    }

    #[test]
    fn dropping_final_pointer_releases_everything() {
        let memory = MemoryBudget::new(1_000);
        let value = TrackedArc::try_new(Value(7), memory.clone()).unwrap();

        drop(value);

        assert_eq!(memory.used(), 0);
        assert_eq!(memory.allocation_bytes(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn pointer_failure_rolls_back_allocation_reservation() {
        let sizing_memory = MemoryBudget::new(1_000);
        let sizing_value = TrackedArc::try_new(Value(7), sizing_memory.clone()).unwrap();
        let allocation_bytes = sizing_memory.allocation_bytes();
        drop(sizing_value);

        let memory = MemoryBudget::new(allocation_bytes);
        assert!(TrackedArc::try_new(Value(7), memory.clone()).is_err());
        assert_eq!(memory.used(), 0);
        assert_eq!(memory.allocation_bytes(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn allocation_failure_never_reserves_a_pointer() {
        let memory = MemoryBudget::new(1);

        assert!(TrackedArc::try_new(Value(7), memory.clone()).is_err());
        assert_eq!(memory.used(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }

    #[test]
    fn tracked_pointer_is_one_machine_pointer_wide() {
        assert_eq!(size_of::<TrackedArc<Value>>(), size_of::<usize>());
    }
}
```

- [ ] **Step 2: Run the construction tests and verify red**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib new::tests
```

Expected: compilation fails because `TrackedArc` and `try_new` do not exist.

- [ ] **Step 3: Add the private allocation and public pointer types**

Append to `crates/memory/src/types.rs`:

```rust
pub struct TrackedArc<T, M: MemoryAccount = MemoryBudget> {
    pub(crate) inner: Arc<Allocation<T, M>>,
}

pub(crate) struct Allocation<T, M: MemoryAccount> {
    pub(crate) value: T,
    pub(crate) memory: M,
    pub(crate) allocation_bytes: u64,
}
```

Update `crates/memory/src/lib.rs`:

```rust
mod access;
mod budget;
mod drop;
mod new;
mod types;

pub use types::{
    ConservativeSize, MemoryAccount, MemoryBudget, MemoryFull, MemoryKind,
    TrackedArc,
};
```

- [ ] **Step 4: Implement conservative sizing and fallible construction**

Implement `crates/memory/src/new.rs` above its test module:

```rust
use std::mem::{size_of, size_of_val};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crate::types::{Allocation, ConservativeSize, MemoryAccount, MemoryKind, TrackedArc};

fn conservative_allocation_bytes<T, M>(value: &T) -> u64
where
    T: ConservativeSize,
    M: MemoryAccount,
{
    let value_bytes = value.conservative_size().max(size_of_val(value) as u64);
    let fixed_fields = size_of::<Allocation<T, M>>().saturating_sub(size_of::<T>()) as u64;
    let arc_counters = size_of::<AtomicUsize>().saturating_mul(2) as u64;
    value_bytes.saturating_add(fixed_fields).saturating_add(arc_counters)
}

impl<T, M> TrackedArc<T, M>
where
    T: ConservativeSize,
    M: MemoryAccount,
{
    pub fn try_new(value: T, memory: M) -> Result<Self, M::Error> {
        let allocation_bytes = conservative_allocation_bytes::<T, M>(&value);
        let pointer_bytes = size_of::<Self>() as u64;
        memory.try_reserve(MemoryKind::Allocation, allocation_bytes)?;
        if let Err(error) = memory.try_reserve(MemoryKind::Pointer, pointer_bytes) {
            memory.release(MemoryKind::Allocation, allocation_bytes);
            return Err(error);
        }
        Ok(Self {
            inner: Arc::new(Allocation { value, memory, allocation_bytes }),
        })
    }
}
```

- [ ] **Step 5: Implement exact drop accounting**

Create `crates/memory/src/drop.rs`:

```rust
use std::mem::size_of;

use crate::types::{Allocation, MemoryAccount, MemoryKind, TrackedArc};

impl<T, M> Drop for TrackedArc<T, M>
where
    M: MemoryAccount,
{
    fn drop(&mut self) {
        self.inner.memory.release(MemoryKind::Pointer, size_of::<Self>() as u64);
    }
}

impl<T, M> Drop for Allocation<T, M>
where
    M: MemoryAccount,
{
    fn drop(&mut self) {
        self.memory.release(MemoryKind::Allocation, self.allocation_bytes);
    }
}
```

- [ ] **Step 6: Implement read-only access without exposing the Arc**

Create `crates/memory/src/access.rs`:

```rust
use std::fmt;
use std::ops::Deref;

use crate::types::{MemoryAccount, TrackedArc};

impl<T, M> Deref for TrackedArc<T, M>
where
    M: MemoryAccount,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner.value
    }
}

impl<T, M> AsRef<T> for TrackedArc<T, M>
where
    M: MemoryAccount,
{
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T, M> fmt::Debug for TrackedArc<T, M>
where
    T: fmt::Debug,
    M: MemoryAccount,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(formatter)
    }
}

impl<T, M> PartialEq for TrackedArc<T, M>
where
    T: PartialEq,
    M: MemoryAccount,
{
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl<T, M> Eq for TrackedArc<T, M>
where
    T: Eq,
    M: MemoryAccount,
{
}
```

- [ ] **Step 7: Run tests, format, and lint**

Run:

```bash
cargo fmt --manifest-path crates/memory/Cargo.toml
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo clippy --manifest-path crates/memory/Cargo.toml --all-targets -- -D warnings
```

Expected: ten budget/construction tests pass and the wrapper-size assertion
proves a one-pointer representation.

- [ ] **Step 8: Commit tracked construction and destruction**

```bash
git add crates/memory
git commit -m "feat(memory): add tracked allocation ownership"
```

---

### Task 3: Fallible pointer cloning and concurrent release

**Files:**
- Modify: `crates/memory/src/lib.rs`
- Create: `crates/memory/src/clone.rs`

**Interfaces:**
- Consumes: `TrackedArc<T, M>` and `MemoryAccount` from Tasks 1 and 2.
- Produces: `TrackedArc::try_clone(&self) -> Result<Self, M::Error>`.

- [ ] **Step 1: Write failing clone and concurrency tests**

Create `crates/memory/src/clone.rs` with these tests before implementation:

```rust
#[cfg(test)]
mod tests {
    use std::mem::size_of;
    use crate::{ConservativeSize, MemoryBudget, TrackedArc};

    struct Value(u64);

    impl ConservativeSize for Value {
        fn conservative_size(&self) -> u64 {
            64
        }
    }

    #[test]
    fn clone_reserves_only_one_additional_pointer() {
        let memory = MemoryBudget::new(1_000);
        let original = TrackedArc::try_new(Value(7), memory.clone()).unwrap();
        let allocation_bytes = memory.allocation_bytes();

        let clone = original.try_clone().unwrap();

        assert_eq!(memory.allocation_bytes(), allocation_bytes);
        assert_eq!(memory.pointer_bytes(), (size_of::<TrackedArc<Value>>() * 2) as u64);
        assert!(std::ptr::eq(original.as_ref(), clone.as_ref()));
        assert_eq!(clone.0, 7);
    }

    #[test]
    fn failed_clone_changes_neither_pointer_count_nor_value() {
        let sizing_memory = MemoryBudget::new(1_000);
        let sizing_value = TrackedArc::try_new(Value(7), sizing_memory.clone()).unwrap();
        let exact_limit = sizing_memory.used();
        drop(sizing_value);

        let memory = MemoryBudget::new(exact_limit);
        let original = TrackedArc::try_new(Value(7), memory.clone()).unwrap();
        let used = memory.used();

        assert!(original.try_clone().is_err());
        assert_eq!(memory.used(), used);
        assert_eq!(memory.pointer_bytes(), size_of::<TrackedArc<Value>>() as u64);
        assert_eq!(original.0, 7);
    }

    #[test]
    fn dropping_non_final_pointer_releases_only_pointer_bytes() {
        let memory = MemoryBudget::new(1_000);
        let original = TrackedArc::try_new(Value(7), memory.clone()).unwrap();
        let clone = original.try_clone().unwrap();
        let allocation_bytes = memory.allocation_bytes();

        drop(clone);

        assert_eq!(memory.allocation_bytes(), allocation_bytes);
        assert_eq!(memory.pointer_bytes(), size_of::<TrackedArc<Value>>() as u64);
        drop(original);
        assert_eq!(memory.used(), 0);
    }

    #[test]
    fn concurrent_pointer_drops_return_all_memory() {
        let memory = MemoryBudget::new(10_000);
        let original = TrackedArc::try_new(Value(7), memory.clone()).unwrap();
        let pointers = (0..32).map(|_| original.try_clone().unwrap()).collect::<Vec<_>>();
        let handles = pointers
            .into_iter()
            .map(|pointer| std::thread::spawn(move || drop(pointer)))
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(memory.pointer_bytes(), size_of::<TrackedArc<Value>>() as u64);
        drop(original);
        assert_eq!(memory.used(), 0);
        assert_eq!(memory.allocation_bytes(), 0);
        assert_eq!(memory.pointer_bytes(), 0);
    }
}
```

- [ ] **Step 2: Run clone tests and verify red**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib clone::tests
```

Expected: compilation fails because `try_clone` is not defined.

- [ ] **Step 3: Implement fallible cloning**

Create `crates/memory/src/clone.rs` above the test module:

```rust
use std::mem::size_of;
use std::sync::Arc;

use crate::types::{MemoryAccount, MemoryKind, TrackedArc};

impl<T, M> TrackedArc<T, M>
where
    M: MemoryAccount,
{
    pub fn try_clone(&self) -> Result<Self, M::Error> {
        self.inner.memory.try_reserve(MemoryKind::Pointer, size_of::<Self>() as u64)?;
        Ok(Self { inner: Arc::clone(&self.inner) })
    }
}
```

Add `mod clone;` to `crates/memory/src/lib.rs`.

- [ ] **Step 4: Run the focused and complete tests under strict lint**

Run:

```bash
cargo fmt --manifest-path crates/memory/Cargo.toml
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib clone::tests
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo clippy --manifest-path crates/memory/Cargo.toml --all-targets -- -D warnings
```

Expected: all tests pass, including the joined-thread accounting test, with no
Clippy warnings.

- [ ] **Step 5: Commit fallible pointer cloning**

```bash
git add crates/memory
git commit -m "feat(memory): add fallible tracked pointer cloning"
```

---

### Task 4: Unit benchmarks, documentation, and isolation audit

**Files:**
- Modify: `crates/memory/src/budget.rs`
- Modify: `crates/memory/src/new.rs`
- Modify: `crates/memory/src/clone.rs`
- Modify: `crates/memory/src/drop.rs`
- Create: `crates/memory/README.md`

**Interfaces:**
- Consumes: all public memory crate operations from Tasks 1–3.
- Produces: reproducible unit benchmark commands and documented measured costs.

- [ ] **Step 1: Add one ignored inline Criterion benchmark per public hot unit**

In each owning file, add an ignored test that constructs a local
`Criterion::default()`, benchmarks the real public operation, calls
`criterion.final_summary()`, and uses `BatchSize::SmallInput` or
`BatchSize::LargeInput` to keep setup outside timing:

```rust
#[test]
#[ignore = "inline Criterion benchmark"]
fn benchmark_budget() {
    let mut criterion = criterion::Criterion::default();
    let memory = MemoryBudget::new(u64::MAX);
    criterion.bench_function("memory/budget/reserve_and_release_pointer", |bencher| {
        bencher.iter(|| {
            memory.try_reserve(MemoryKind::Pointer, 8).unwrap();
            memory.release(MemoryKind::Pointer, 8);
        });
    });
    criterion.final_summary();
}
```

Use these exact benchmark names for the other units:

- `memory/tracked_arc/try_new`
- `memory/tracked_arc/try_clone`
- `memory/tracked_arc/drop_non_final`
- `memory/tracked_arc/drop_final`

For `try_clone`, prepare the source pointer and enough budget outside the timed
closure. Return the successful cloned pointer from the closure so Criterion
drops it after timing. For non-final drop, prepare `(original, clone)` and time
only `drop(clone)`. For final drop, prepare one pointer and time `drop(pointer)`.
Use the same 64-byte conservative fixture in every tracked-pointer benchmark.

- [ ] **Step 2: Run all correctness tests before timing**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --lib
```

Expected: every non-ignored test passes before benchmark measurements begin.

- [ ] **Step 3: Run each inline benchmark and capture its median**

Run:

```bash
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_budget -- --ignored --nocapture --test-threads=1
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_try_new -- --ignored --nocapture --test-threads=1
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_try_clone -- --ignored --nocapture --test-threads=1
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_drop -- --ignored --nocapture --test-threads=1
```

Expected: every command passes and Criterion reports a stable time interval for
its named unit.

- [ ] **Step 4: Document the crate and the observed results**

Create `crates/memory/README.md` with:

- the isolation statement;
- the allocation-versus-pointer accounting model;
- the `try_new` rollback contract;
- the lack of an infallible `Clone` implementation;
- the exact drop order;
- the five verification commands above; and
- a benchmark table whose rows are the five exact benchmark names and whose
  values are the medians printed in Step 3.

State explicitly that the measurements exclude the future API, router,
worker, event-history, checkpoint, and lane pipeline.

- [ ] **Step 5: Run the final verification and dependency audit**

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

Expected:

- all correctness tests pass;
- doc tests pass;
- strict Clippy and formatting pass;
- `cargo tree -e normal` shows only `contime-memory` with no child dependency;
- the diff check is clean.

- [ ] **Step 6: Commit the verified isolated crate**

```bash
git add crates/memory
git commit -m "docs(memory): record tracked pointer benchmarks"
```
