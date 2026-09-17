# Isolated Router Subcrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an isolated `contime-router` crate that receives complete input batches, deterministically partitions their snapshot routes, and sends one batch to each affected worker.

**Architecture:** A single blocking `route` function owns no persistent object state. It derives AHash state from a caller-provided `u64` seed, receives router-local `InputBatch` values through Crossbeam, flattens inputs directly into final per-worker vectors, and sends router-local `WorkerBatch` values through injected Crossbeam worker senders. The root `contime` crate and `contime-api` remain unchanged and do not depend on the new crate during this pass.

**Tech Stack:** Rust 2021, `ahash` 0.8, `crossbeam-channel` 0.5, inline Criterion 0.5 benchmarks.

**Spec:** `docs/superpowers/specs/2026-08-28-router-subcrate-design.md`

## Global Constraints

- Create files only under `crates/router/`; do not modify root-crate source, workspace configuration, or `crates/api/`.
- Do not perform Git operations or create commits.
- The router must not depend on `contime` or `contime-api`.
- Input batches are received through a Crossbeam `Receiver`.
- Worker batches are sent through injected Crossbeam `Sender` values.
- The caller supplies one `u64` seed; hashing is a private router implementation detail.
- The same seed, worker count, snapshot IDs, and router algorithm version must produce the same worker assignment across route runs.
- The router owns no threads, workers, memory accounting, query routing, time advancement, rejection construction, response waiting, or recovery orchestration.
- All public types live in `src/types.rs`; the one public function lives in `src/route.rs` and is re-exported by `src/lib.rs`.
- Use inline unit tests and inline ignored Criterion benchmarks only; do not create an integration-test or external benchmark directory.

---

## File Map

- Create `crates/router/.gitignore`: ignore the standalone crate's `target/` and `Cargo.lock`.
- Create `crates/router/Cargo.toml`: standalone crate metadata and dependencies.
- Create `crates/router/README.md`: responsibilities, exclusions, verification commands, and measured benchmark results.
- Create `crates/router/src/lib.rs`: module declarations and public re-exports only.
- Create `crates/router/src/types.rs`: `RoutableInput`, `InputBatch`, `RoutedInput`, `WorkerBatch`, and `RouterError`.
- Create `crates/router/src/route.rs`: created test-first in Task 2; contains `route`, private routing helpers, the file-local dependency seam, unit tests, and the inline Criterion benchmark.

---

### Task 1: Scaffold the Standalone Crate and Public Contracts

**Files:**
- Create: `crates/router/.gitignore`
- Create: `crates/router/Cargo.toml`
- Create: `crates/router/README.md`
- Create: `crates/router/src/lib.rs`
- Create: `crates/router/src/types.rs`

**Interfaces:**
- Consumes: no existing crate interface.
- Produces: router-local public types; Task 2 adds the public function test-first.

- [ ] **Step 1: Create the standalone crate metadata and ignore file**

Create `crates/router/Cargo.toml`:

```toml
[package]
name = "contime-router"
version = "0.1.0"
edition = "2021"
autobenches = false
license = "MIT"
description = "Isolated deterministic input routing for ConTime"
publish = false

[dependencies]
ahash = "0.8.12"
crossbeam-channel = "0.5"

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
```

Create `crates/router/.gitignore`:

```gitignore
/target/
/Cargo.lock
```

- [ ] **Step 2: Define all public types**

Create `crates/router/src/types.rs`:

```rust
#[derive(Debug)]
pub struct InputBatch<I, C> {
    pub inputs: Vec<I>,
    pub completion: C,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RoutedInput<I> {
    pub snapshot_id: u128,
    pub input: I,
}

#[derive(Debug)]
pub struct WorkerBatch<I, C> {
    pub inputs: Vec<RoutedInput<I>>,
    pub completion: C,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouterError {
    NoWorkers,
    WorkerUnavailable { worker_index: usize },
}

pub trait RoutableInput: Clone {
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128));
}
```

- [ ] **Step 3: Declare the initial type re-exports**

Create `crates/router/src/lib.rs` without a route module yet:

```rust
//! Deterministic input-batch routing independent of ConTime orchestration.

mod types;

pub use types::{InputBatch, RoutableInput, RoutedInput, RouterError, WorkerBatch};
```

- [ ] **Step 4: Add the initial README boundary statement**

Create `crates/router/README.md` with these sections and facts:

```markdown
# contime-router

`contime-router` receives complete input batches, deterministically maps each
snapshot route to a worker, and sends one final batch per affected worker.

The crate is currently isolated: the root `contime` crate does not use it, and
it is not a workspace member. It has no dependency on `contime` or
`contime-api`.

## Responsibilities

- Receive router-local input batches.
- Hash snapshot IDs from a caller-provided seed.
- Flatten directly into final worker vectors.
- Send one batch per affected worker.

## Exclusions

The crate owns no threads, workers, memory accounting, queries, time
advancement, rejection semantics, response waiting, or recovery orchestration.
```

- [ ] **Step 5: Verify the standalone crate compiles**

Run:

```bash
cargo check --manifest-path crates/router/Cargo.toml
```

Expected: `contime-router` compiles successfully without compiling or modifying the root `contime` crate.

---

### Task 2: Implement Receiver Lifecycle and Error Boundaries Test-First

**Files:**
- Modify: `crates/router/src/route.rs`

**Interfaces:**
- Consumes: `InputBatch<I, C>`, `RoutableInput`, `RouterError`, and `WorkerBatch<I, C>` from Task 1.
- Produces: a blocking `route` loop that rejects zero workers and treats input disconnection as normal shutdown. Task 3 adds worker dispatch failures.

- [ ] **Step 1: Create `route.rs` with failing lifecycle tests before production code**

Create `route.rs` with imports and an inline test module, but do not define `route` yet:

```rust
#[cfg(test)]
mod tests {
    use crossbeam_channel::unbounded;

    use super::route;
    use crate::{InputBatch, RoutableInput, RouterError};

    #[derive(Clone, Debug, PartialEq, Eq)]
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
    fn route_rejects_an_empty_worker_list() {
        let (_input_sender, input_receiver) = unbounded::<InputBatch<TestInput, ()>>();

        let result = route(7, input_receiver, &[]);

        assert_eq!(result, Err(RouterError::NoWorkers));
    }

    #[test]
    fn route_stops_normally_when_input_disconnects() {
        let (input_sender, input_receiver) = unbounded::<InputBatch<TestInput, ()>>();
        let (worker_sender, _worker_receiver) = unbounded();
        drop(input_sender);

        let result = route(7, input_receiver, &[worker_sender]);

        assert_eq!(result, Ok(()));
    }
}
```

- [ ] **Step 2: Export the still-missing function and verify RED**

Add `mod route;` and `pub use route::route;` to `src/lib.rs`, then run:

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib
```

Expected: compilation FAILS because `super::route` and the re-exported `route` function do not exist. This is the expected RED state.

- [ ] **Step 3: Add the minimal public function, receive loop, and private batch hook**

Add these imports and production definitions above the test module:

```rust
use crossbeam_channel::{Receiver, Sender};

use crate::types::{InputBatch, RoutableInput, RouterError, WorkerBatch};

pub fn route<I, C>(
    seed: u64,
    input: Receiver<InputBatch<I, C>>,
    worker_outputs: &[Sender<WorkerBatch<I, C>>],
) -> Result<(), RouterError>
where
    I: RoutableInput,
    C: Clone,
{
    if worker_outputs.is_empty() {
        return Err(RouterError::NoWorkers);
    }

    let hasher = hasher_from_seed(seed);
    while let Ok(batch) = input.recv() {
        route_batch(&hasher, batch, worker_outputs)?;
    }
    Ok(())
}

fn route_batch<I, C>(
    _hasher: &ahash::RandomState,
    _batch: InputBatch<I, C>,
    _worker_outputs: &[Sender<WorkerBatch<I, C>>],
) -> Result<(), RouterError>
where
    I: RoutableInput,
    C: Clone,
{
    Ok(())
}
```

Add a private deterministic hasher constructor:

```rust
fn hasher_from_seed(seed: u64) -> ahash::RandomState {
    ahash::RandomState::with_seeds(
        seed,
        seed.rotate_left(17),
        seed.rotate_left(33),
        seed.rotate_left(49),
    )
}
```

- [ ] **Step 4: Run all current unit tests and verify GREEN**

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib
```

Expected: both lifecycle tests pass with no warnings.

---

### Task 3: Partition Directly Into Final Worker Batches Test-First

**Files:**
- Modify: `crates/router/src/route.rs`

**Interfaces:**
- Consumes: the blocking receiver loop and seeded `RandomState` from Task 2.
- Produces: direct one-pass partitioning, deterministic worker mapping, minimal input and completion cloning, and one send per affected worker.

- [ ] **Step 1: Write a failing single-route dispatch test**

Add a test helper that preloads one input batch, disconnects the input channel, runs `route`, and drains all worker receivers:

```rust
fn route_once(
    seed: u64,
    inputs: Vec<TestInput>,
    worker_count: usize,
) -> Vec<(usize, crate::WorkerBatch<TestInput, ()>)> {
    let (input_sender, input_receiver) = unbounded();
    input_sender.send(InputBatch { inputs, completion: () }).unwrap();
    drop(input_sender);

    let mut worker_outputs = Vec::with_capacity(worker_count);
    let mut worker_receivers = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let (worker_sender, worker_receiver) = unbounded();
        worker_outputs.push(worker_sender);
        worker_receivers.push(worker_receiver);
    }

    route(seed, input_receiver, &worker_outputs).unwrap();

    worker_receivers
        .into_iter()
        .enumerate()
        .flat_map(|(worker_index, receiver)| {
            receiver.try_iter().map(move |batch| (worker_index, batch))
        })
        .collect()
}
```

Then add:

```rust
#[test]
fn route_dispatches_every_snapshot_route_once() {
    let batches = route_once(
        7,
        vec![
            TestInput { value: 10, snapshot_ids: vec![11] },
            TestInput { value: 20, snapshot_ids: vec![22] },
            TestInput { value: 30, snapshot_ids: vec![33] },
        ],
        4,
    );

    let mut routed = batches
        .into_iter()
        .flat_map(|(_worker_index, batch)| batch.inputs)
        .map(|routed| (routed.snapshot_id, routed.input.value))
        .collect::<Vec<_>>();
    routed.sort_unstable();

    assert_eq!(routed, vec![(11, 10), (22, 20), (33, 30)]);
}
```

- [ ] **Step 2: Run the dispatch test and verify RED**

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib route_dispatches_every_snapshot_route_once
```

Expected: FAIL because Task 2 drops every received batch and produces no worker batches.

- [ ] **Step 3: Add the private output dependency seam**

Inside `route.rs`, define a file-local trait:

```rust
trait Deps<I, C> {
    fn worker_count(&self) -> usize;
    fn send(&self, worker_index: usize, batch: WorkerBatch<I, C>) -> Result<(), ()>;
}
```

Implement `DefaultDeps` over `&[Sender<WorkerBatch<I, C>>]`. Its `send` method calls the selected Crossbeam sender and maps `SendError` to `()`.

Make public `route` call private `route_with_deps(seed, input, &DefaultDeps { worker_outputs })`. This preserves one production code path while allowing later unit tests to stub delivery.

- [ ] **Step 4: Implement direct worker-vector partitioning**

Use `std::hash::BuildHasher` and this worker mapping:

```rust
fn worker_index(hasher: &ahash::RandomState, snapshot_id: u128, worker_count: usize) -> usize {
    hasher.hash_one(snapshot_id) as usize % worker_count
}
```

For each batch, allocate worker slots with exact outer length:

```rust
let worker_count = deps.worker_count();
let base_capacity = batch.inputs.len().div_ceil(worker_count);
let estimated_capacity = base_capacity
    .saturating_add(base_capacity / 4)
    .saturating_add(1);
let mut worker_inputs = Vec::with_capacity(worker_count);
worker_inputs.resize_with(worker_count, || None::<Vec<RoutedInput<I>>>);
```

For every input, retain one pending snapshot ID. Clone into every prior route and move into the final route:

```rust
let mut pending_snapshot_id = None;
input.snapshot_ids(&mut |snapshot_id| {
    if let Some(previous_snapshot_id) = pending_snapshot_id.replace(snapshot_id) {
        push_route(
            &mut worker_inputs,
            hasher,
            worker_count,
            estimated_capacity,
            previous_snapshot_id,
            input.clone(),
        );
    }
});
if let Some(final_snapshot_id) = pending_snapshot_id {
    push_route(
        &mut worker_inputs,
        hasher,
        worker_count,
        estimated_capacity,
        final_snapshot_id,
        input,
    );
}
```

`push_route` lazily creates only an affected worker's inner vector and pushes `RoutedInput { snapshot_id, input }`.

Count non-empty worker vectors. Clone `batch.completion` for all but the final affected worker, move the original into the final batch, and call `deps.send` exactly once per affected worker. Map a failed send to `RouterError::WorkerUnavailable { worker_index }`.

- [ ] **Step 5: Run the single-route test and verify GREEN**

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib route_dispatches_every_snapshot_route_once
```

Expected: PASS.

- [ ] **Step 6: Add deterministic routing and ordering tests**

Add tests that run identical inputs twice with seed `7` and eight workers, record the worker index containing each snapshot ID, and assert both maps are equal. Add a second test whose inputs all resolve to the same worker and assert their `value` fields remain in caller order.

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib deterministic
cargo test --manifest-path crates/router/Cargo.toml --lib preserves_input_order
```

Expected: both tests pass.

- [ ] **Step 7: Add exact input-clone coverage**

Define a test input with `clone_count: Arc<AtomicUsize>` and a manual `Clone` implementation that increments the counter. Route one input emitting three snapshot IDs and assert the clone counter equals `2` after routing.

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib clones_an_input_only_for_additional_snapshot_ids
```

Expected: PASS with exactly two clones.

- [ ] **Step 8: Add exact completion-clone coverage**

Define a completion token with a manual `Clone` implementation backed by `Arc<AtomicUsize>`. Select snapshot IDs that map to three distinct workers using the private `worker_index` helper, route one batch, and assert the completion clone counter equals `2`.

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib clones_completion_only_for_additional_workers
```

Expected: PASS with exactly two clones.

- [ ] **Step 9: Add zero-route and worker-disconnection coverage**

Add one test with inputs whose `snapshot_ids` vectors are empty and assert every worker receiver remains empty after `route` returns. Add another test that drops a selected worker receiver before routing an ID mapped to that worker and assert:

```rust
Err(RouterError::WorkerUnavailable { worker_index: selected_worker })
```

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib
```

Expected: all router unit tests pass with no ignored tests other than the Criterion benchmark introduced in Task 4.

---

### Task 4: Add and Run the Inline Router Benchmark

**Files:**
- Modify: `crates/router/src/route.rs`
- Modify: `crates/router/README.md`

**Interfaces:**
- Consumes: the complete production `route` function from Task 3.
- Produces: a reproducible 1,000-input, eight-worker Criterion measurement and documented interpretation.

- [ ] **Step 1: Add the ignored inline Criterion benchmark**

Inside the existing `#[cfg(test)]` module, add a `BenchmarkInput` with one snapshot ID and:

```rust
#[test]
#[ignore = "inline Criterion benchmark"]
fn benchmark_route() {
    let mut criterion = criterion::Criterion::default();

    criterion.bench_function("router/1000_inputs/8_workers", |bencher| {
        bencher.iter_batched(
            || benchmark_fixture(1_000, 8),
            |(input_receiver, worker_outputs, worker_receivers)| {
                route(7, input_receiver, &worker_outputs).unwrap();
                std::hint::black_box(worker_receivers)
            },
            criterion::BatchSize::LargeInput,
        );
    });

    criterion.final_summary();
}
```

`benchmark_fixture` must construct the 1,000 inputs, input channel, and eight worker channel pairs outside the timed routine; enqueue one complete input batch; drop the input sender; and return the input receiver, worker senders, and worker receivers.

- [ ] **Step 2: Run normal unit tests**

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --lib
```

Expected: every behavior test passes and exactly one inline Criterion test is ignored.

- [ ] **Step 3: Run the release benchmark**

Run:

```bash
cargo test --manifest-path crates/router/Cargo.toml --release --lib benchmark_route -- --ignored --nocapture
```

Expected: Criterion reports a confidence interval for `router/1000_inputs/8_workers` and the ignored test exits successfully.

- [ ] **Step 4: Document the verified benchmark**

Extend `crates/router/README.md` with:

- the normal unit-test command;
- the exact benchmark command;
- the exact confidence interval printed in Step 3;
- the seed (`7`), input count (`1,000`), and worker count (`8`);
- a statement that input/channel construction and worker processing are excluded;
- a statement that receipt, hasher derivation, snapshot visitation, hashing, routing-vector allocation, completion cloning, and real worker-channel sends are included.

- [ ] **Step 5: Run final formatting and verification**

Run:

```bash
cargo fmt --manifest-path crates/router/Cargo.toml -- --check
cargo test --manifest-path crates/router/Cargo.toml --lib
cargo test --manifest-path crates/router/Cargo.toml --release --lib benchmark_route -- --ignored --nocapture
```

Expected: formatting succeeds, all behavior tests pass, and the fresh Criterion interval matches the documented benchmark within normal measurement variance. If the fresh interval falls outside the documented range, replace the README range with the fresh final interval.

Verify scope with:

```bash
git status --short --untracked-files=all
```

Expected: this implementation adds only `crates/router/` files; the pre-existing design and plan documents are the only documentation changes outside that directory, and no root-crate source file changed during execution.
