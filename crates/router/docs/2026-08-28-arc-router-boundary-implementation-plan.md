# Arc-only Router Boundary Implementation Plan

> Historical implementation plan: superseded by the ownership-generic router
> boundary on 2026-08-31. It remains only as a record of the earlier design.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Require `Arc`-owned events at the isolated router boundary while preserving deterministic routing, completion behavior, and the single-worker hash shortcut.

**Architecture:** `InputBatch<E, C>` owns `Vec<Arc<E>>`, and every `RoutedInput<E>` owns one `Arc<E>` plus its resolved `u128` snapshot ID. The router moves the original Arc into the final route and calls `Arc::clone` only for preceding additional routes, so workers receive exact, independently owned batches without revisiting snapshot IDs.

**Tech Stack:** Rust, `std::sync::Arc`, Crossbeam channels, Criterion, AHash.

**Spec:** `crates/router/docs/2026-08-28-arc-router-boundary-design.md`

## Global Constraints

- Change only `crates/router`; do not modify the root `contime` crate.
- Do not perform git operations.
- Preserve the public `route(seed, input, worker_outputs)` function shape apart from its Arc-enforced batch types.
- Preserve deterministic worker assignment, input order within worker batches, completion semantics, error behavior, and the one-worker no-hash shortcut.
- Do not add an indexed shared-vector representation, snapshot-ID deduplication, sorting, or worker-side route resolution.
- Keep tests inline; do not add an integration-test directory.

---

### Task 1: Encode Arc ownership in the public batch types

**Files:**
- Modify: `crates/router/src/types.rs`
- Modify: `crates/router/src/route.rs`

**Interfaces:**
- Consumes: existing `RoutableInput::snapshot_ids`, `InputBatch`, `RoutedInput`, and `WorkerBatch` APIs.
- Produces: `InputBatch<E, C> { inputs: Vec<Arc<E>>, completion: C }`, `RoutedInput<E> { snapshot_id: u128, input: Arc<E> }`, and a `RoutableInput` trait without a `Clone` supertrait.

- [ ] **Step 1: Add a failing non-Clone event test**

In `route.rs` tests, replace the test event's derived `Clone` requirement with a deliberately non-Clone event and construct the input batch from Arcs:

```rust
#[derive(Debug, PartialEq, Eq)]
struct TestInput {
    value: u64,
    snapshot_ids: Vec<u128>,
}

impl RoutableInput for TestInput {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        self.snapshot_ids.iter().copied().for_each(emit);
    }
}

#[test]
fn route_accepts_a_non_clone_event_behind_arc() {
    let (input_sender, input_receiver) = unbounded();
    let (worker_sender, worker_receiver) = unbounded();
    input_sender
        .send(InputBatch {
            inputs: vec![Arc::new(TestInput {
                value: 10,
                snapshot_ids: vec![11],
            })],
            completion: (),
        })
        .unwrap();
    drop(input_sender);

    route(7, input_receiver, &[worker_sender]).unwrap();

    let routed = worker_receiver.recv().unwrap().inputs.pop().unwrap();
    assert_eq!(routed.snapshot_id, 11);
    assert_eq!(routed.input.value, 10);
}
```

- [ ] **Step 2: Run the focused test and verify the Arc boundary is missing**

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib route_accepts_a_non_clone_event_behind_arc
```

Expected: compilation fails because the current `RoutableInput: Clone` contract rejects `TestInput`, demonstrating that the old boundary is still active.

- [ ] **Step 3: Change the public types to own Arcs**

In `types.rs`, import `std::sync::Arc` and use:

```rust
pub struct InputBatch<E, C> {
    pub inputs: Vec<Arc<E>>,
    pub completion: C,
}

pub struct RoutedInput<E> {
    pub snapshot_id: u128,
    pub input: Arc<E>,
}

pub struct WorkerBatch<E, C> {
    pub inputs: Vec<RoutedInput<E>>,
    pub completion: C,
}

pub trait RoutableInput {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}
```

Keep `RouterError` unchanged.

- [ ] **Step 4: Update routing internals without introducing another path**

In `route.rs`, keep `route`, `route_with_deps`, `route_batch`, and `push_route` generic over the underlying event `E`. Change only the routed value types:

```rust
fn push_route<E>(
    worker_inputs: &mut [Option<Vec<RoutedInput<E>>>],
    hasher: &RouterHasher,
    worker_count: usize,
    estimated_capacity: usize,
    snapshot_id: u128,
    input: Arc<E>,
) {
    let worker_index = hasher.worker_index(snapshot_id, worker_count);
    worker_inputs[worker_index]
        .get_or_insert_with(|| Vec::with_capacity(estimated_capacity))
        .push(RoutedInput { snapshot_id, input });
}
```

Import `std::sync::Arc`. In `route_batch`, retain the pending-final-snapshot algorithm, replace `input.clone()` with `Arc::clone(&input)`, and move `input` into the final route. Do not change capacity estimation, worker selection, completion cloning, or sending.

- [ ] **Step 5: Update existing unit fixtures to construct Arc events**

Change `route_once` to accept `Vec<Arc<TestInput>>` and return `WorkerBatch<TestInput, ()>`. Update every existing fixture to wrap each event with `Arc::new`. Where deterministic-assignment tests need a second batch, create two separately constructed Arc vectors rather than requiring `TestInput: Clone`.

- [ ] **Step 6: Replace custom event-clone tests with Arc strong-count tests**

Delete `CloneCountInput`. Add tests that retain only a `Weak<TestInput>` outside the router:

```rust
#[test]
fn one_route_moves_the_only_arc_without_cloning() {
    let event = Arc::new(TestInput { value: 10, snapshot_ids: vec![11] });
    let weak = Arc::downgrade(&event);
    // Send `event`, route it, and retain the worker batch in its receiver.
    assert_eq!(weak.strong_count(), 1);
}

#[test]
fn additional_routes_clone_the_arc_once_each() {
    let event = Arc::new(TestInput {
        value: 10,
        snapshot_ids: vec![11, 22, 33],
    });
    let weak = Arc::downgrade(&event);
    // Send `event`, route it, and retain all worker batches in their receivers.
    assert_eq!(weak.strong_count(), 3);
}
```

The full tests must also assert that exactly three routed inputs exist, so the strong-count assertion cannot pass because of an unrelated retained Arc.

- [ ] **Step 7: Run the complete library tests**

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib
```

Expected: all behavioral tests pass, including non-Clone event acceptance, one-route strong count `1`, additional-route strong count `3`, completion cloning, deterministic assignment, zero routes, unavailable workers, and single-worker selection.

- [ ] **Step 8: Review checkpoint**

Inspect `types.rs` and `route.rs` together. Confirm no generic owned-event path remains, no event type requires `Clone`, and the only event clone operation is `Arc::clone` for additional snapshot IDs.

---

### Task 2: Convert executable routing benchmarks to Arc-only inputs

**Files:**
- Modify: `crates/router/benches/router.rs`

**Interfaces:**
- Consumes: the Arc-only `InputBatch<E, C>` and `WorkerBatch<E, C>` from Task 1.
- Produces: executable Criterion routing benchmarks that exercise only the supported Arc boundary, while retaining isolated `Arc::new` measurements.

- [ ] **Step 1: Compile the existing benches against the Arc-only API and observe failure**

Run:

```bash
cargo check --manifest-path crates/router/Cargo.toml --benches
```

Expected: compilation fails where fixtures still construct owned `InputBatch` values or use the benchmark-only shared wrapper as the routed event type.

- [ ] **Step 2: Remove benchmark-only ownership wrappers and owned routing cases**

Delete `SharedBenchmarkInput`. Keep `BenchmarkEvent32`, `BenchmarkEvent<PAYLOAD_BYTES>`, `SnapshotIds`, and their `RoutableInput` implementations as underlying non-Clone event types. Make fixture input vectors `Vec<Arc<E>>` and worker channels `WorkerBatch<E, Completion>`.

Use direct Arc construction in setup:

```rust
fn arc_fixture<const PAYLOAD_BYTES: usize>(route_count: usize) -> Fixture<BenchmarkEvent<PAYLOAD_BYTES>> {
    fixture(
        benchmark_events::<PAYLOAD_BYTES>(route_count)
            .map(Arc::new)
            .collect(),
    )
}
```

Remove executable `owned` routing cases. Rename the remaining route benchmark IDs so they state route count without implying a choice the public API no longer offers, for example `1_route` and `2_routes`.

- [ ] **Step 3: Update exact-size assertions**

Keep assertions that the underlying events are exactly 32, 64, 208, and 1,008 bytes where applicable. Assert that routed Arc records have the expected 32-byte layout:

```rust
const _: () = assert!(std::mem::size_of::<RoutedInput<BenchmarkEvent32>>() == 32);
const _: () = assert!(std::mem::size_of::<RoutedInput<BenchmarkEvent<0>>>() == 32);
```

- [ ] **Step 4: Preserve focused benchmark coverage**

Keep these executable groups:

- 32-byte event, one route, eight workers;
- 64-byte event, one and two routes, eight workers;
- 64-byte event, one route, one worker;
- isolated `Arc::new` for 32, 64, 208, and 1,008-byte values.

Do not restore the temporarily disabled slow matrix unless separately requested.

- [ ] **Step 5: Compile and run the focused routing benchmarks**

Run:

```bash
cargo check --manifest-path crates/router/Cargo.toml --benches
cargo bench --manifest-path crates/router/Cargo.toml --bench router -- 32_byte_events
cargo bench --manifest-path crates/router/Cargo.toml --bench router -- 64_byte_events
```

Expected: benchmark compilation succeeds; every routing measurement uses `Vec<Arc<E>>`; the one-worker case remains measurably faster than hashing the same routes across the general worker-selection path.

- [ ] **Step 6: Review checkpoint**

Search `benches/router.rs` for `owned` and `SharedBenchmarkInput`. Expected: neither remains in executable benchmark code. Confirm `Arc::new` occurs only in Criterion setup closures for routing measurements and inside the isolated allocation measurement's timed closure.

---

### Task 3: Update documentation and perform final focused verification

**Files:**
- Modify: `crates/router/README.md`
- Verify: `crates/router/docs/2026-08-28-arc-router-boundary-design.md`

**Interfaces:**
- Consumes: final Arc-only API and fresh Criterion results from Tasks 1 and 2.
- Produces: accurate consumer guidance, reproducible commands, and a verified isolated router crate.

- [ ] **Step 1: Update the responsibility and ownership documentation**

Replace language describing ownership as a consumer choice with the Arc-only contract:

```markdown
Every input event enters the router in `Arc`. A single snapshot route moves the
existing Arc without cloning it; each additional snapshot route clones only the
pointer. Workers receive exact `{ snapshot_id, Arc<Event> }` records and never
revisit or rehash an event's snapshot IDs.
```

State explicitly that the underlying event only implements `RoutableInput` and does not need `Clone`.

- [ ] **Step 2: Preserve owned measurements as historical decision evidence**

Keep the recorded owned-versus-shared table, but label owned rows as historical and no longer executable through the Arc-only API. Replace current benchmark instructions and descriptions with the actual Arc-only Criterion IDs from Task 2. Record fresh medians for the enabled Arc routing cases and the single-worker shortcut.

- [ ] **Step 3: Verify formatting and all crate targets**

Run:

```bash
cargo fmt --manifest-path crates/router/Cargo.toml --check
cargo test --manifest-path crates/router/Cargo.toml
cargo check --manifest-path crates/router/Cargo.toml --benches
```

Expected: formatting exits zero; all non-ignored unit and doc tests pass; all bench targets compile.

- [ ] **Step 4: Verify the public boundary from source**

Run:

```bash
rg -n "Vec<Arc|Arc<E>|trait RoutableInput|Arc::clone|\.clone\(\)" crates/router/src crates/router/benches/router.rs
```

Expected: input and routed event fields are Arc-owned; `RoutableInput` has no `Clone` bound; event fan-out uses explicit `Arc::clone`; remaining `.clone()` calls are completion-handle cloning or test setup rather than cloning underlying events.

- [ ] **Step 5: Final review checkpoint**

Compare implementation, README, benchmarks, and tests against every section of the approved specification. Report exact test counts and fresh benchmark medians. Do not modify the root crate and do not perform git operations.
