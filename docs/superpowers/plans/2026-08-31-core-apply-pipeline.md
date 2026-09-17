# Core Apply Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an isolated, memory-accounted, apply-only `contime-core` crate that composes every completed ConTime subcrate.

**Architecture:** Core-owned adapter types implement adjacent producer and consumer traits, while a global atomic budget is cloned into tracked event and checkpoint ownership. The runtime owns only thread execution and channels; domain events, histories, replay, and memory policy stay in their owning adapters.

**Tech Stack:** Rust 2021, crossbeam-channel, Criterion, and the local `contime-{api,router,runtime,worker,events,checkpoints,lanes,memory}` crates.

**Spec:** `docs/superpowers/specs/2026-08-31-core-apply-pipeline-design.md`

## Global Constraints

- Apply only; do not add query, advance, pruning, macros, or integration benchmarks.
- Keep `contime-core` isolated from the root `contime` crate.
- Keep all public types in `src/types.rs` and `src/lib.rs` limited to declarations and re-exports.
- Add inline unit tests and ignored inline Criterion benchmarks to each executable unit.
- Do not commit any changes.

---

### Task 1: Mutable replay acknowledgement

**Files:**
- Modify: `crates/worker/src/types.rs`
- Modify: `crates/worker/src/checkpoints.rs`

**Interfaces:**
- Consumes: `Checkpoints<S>::update` and worker-owned snapshot slots.
- Produces: `fn update(&mut self, events: &mut S, context: &mut Self::Context)`.

- [ ] Add a worker unit test whose checkpoint adapter mutates event history while replaying.
- [ ] Run the focused test and verify the immutable contract fails to compile.
- [ ] Change only the checkpoint update contract and call site to mutable history.
- [ ] Run worker tests and all-target compilation.

### Task 2: Runtime receives assembled processes

**Files:**
- Modify: `crates/runtime/src/types.rs`
- Modify: `crates/runtime/src/start.rs`
- Modify: `crates/runtime/src/runtime.rs`
- Modify: `crates/runtime/src/shutdown.rs`
- Modify: `crates/runtime/README.md`
- Modify: `crates/runtime/benches/*.rs`

**Interfaces:**
- Consumes: `Vec<R: Router>` and `Vec<W: Worker>`.
- Produces: one shared router input sender and worker-specific output queues.

- [ ] Add tests proving supplied router and worker instances run and routers compete on one shared receiver.
- [ ] Run the focused tests and verify the old factory/indexed-input API fails.
- [ ] Replace factories and per-router inputs with supplied instances and one shared input queue.
- [ ] Update existing tests and benchmark compilation without changing benchmark intent.
- [ ] Run runtime tests and all-target compilation.

### Task 3: Core memory budget

**Files:**
- Create: `crates/core/Cargo.toml`
- Create: `crates/core/src/lib.rs`
- Create: `crates/core/src/types.rs`
- Create: `crates/core/src/memory.rs`

**Interfaces:**
- Consumes: `SizeDelta` and `TrackedMemoryBudget`.
- Produces: cloneable `MemoryBudget`, `used_memory`, admission checks, and configured buffer behavior.

- [ ] Write tests for increases, decreases, saturation, and buffer-aware admission.
- [ ] Run the focused test and verify the core crate lacks the API.
- [ ] Implement atomic accounting with relaxed ordering and conservative saturation.
- [ ] Add an inline benchmark for 1,000 balanced delta applications.
- [ ] Run focused tests.

### Task 4: Input admission and tracked ownership

**Files:**
- Create: `crates/core/src/input.rs`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: owned `Input` values and `MemoryBudget`.
- Produces: `TrackedEvent<I>` values implementing canonical event and routing contracts.

- [ ] Write tests proving accepted events are tracked, rejected batches remain unwrapped, clones count only pointers, and final drops restore memory.
- [ ] Run the focused tests and verify the types/functions are absent.
- [ ] Implement whole-batch conservative admission and tracked wrapping.
- [ ] Add an inline benchmark for preparing 1,000 accepted inputs.
- [ ] Run focused tests.

### Task 5: Zero-copy transport adapters

**Files:**
- Create: `crates/core/src/message.rs`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: API output, router input/output, and worker input traits.
- Produces: one API-to-router batch, routed event, worker batch, and completion adapter.

- [ ] Write tests proving the same tracked allocation crosses all adapters and success closes the rejection channel.
- [ ] Run the focused tests and verify adapter traits are unimplemented.
- [ ] Implement the adjacent traits directly on core-owned message structs.
- [ ] Add an inline benchmark for constructing 1,000 routes and one worker batch.
- [ ] Run focused tests.

### Task 6: Canonical history adapter

**Files:**
- Create: `crates/core/src/history.rs`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: `TrackedEvent<I>` and `contime-events::EventHistory`.
- Produces: worker event storage and checkpoint-compatible borrowed raw-event iteration.

- [ ] Write tests for insertion, duplicate no-op behavior, canonical raw borrowing, dirty time, and replay acknowledgement.
- [ ] Run focused tests and verify the adapter is absent.
- [ ] Implement worker and checkpoint event-store traits without payload cloning.
- [ ] Add an inline benchmark for 1,000 ordered inserts.
- [ ] Run focused tests.

### Task 7: Memory-tracked checkpoint adapter

**Files:**
- Create: `crates/core/src/checkpoint.rs`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: canonical history, checkpoint replay, consumer snapshot/apply traits, and tracked owned storage.
- Produces: worker checkpoint storage that acknowledges history and reports snapshot-size deltas.

- [ ] Write tests proving replay materializes state, acknowledges history, and updates/reclaims tracked checkpoint memory.
- [ ] Run focused tests and verify the adapter is absent.
- [ ] Implement conservative checkpoint sizing and tracked replay mutation.
- [ ] Add an inline benchmark for replaying one 1,000-event timestamp bucket.
- [ ] Run focused tests.

### Task 8: Router and worker process adapters

**Files:**
- Create: `crates/core/src/router.rs`
- Create: `crates/core/src/worker.rs`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: core messages, router seed, worker scheduling config, history config, checkpoint config, and apply wrapper.
- Produces: runtime-compatible router and worker instances.

- [ ] Write tests proving each adapter drives its isolated sibling function with real channels.
- [ ] Run focused tests and verify process adapters are absent.
- [ ] Implement runtime `Router` and `Worker` traits through direct sibling calls.
- [ ] Add one inline benchmark per process adapter using pre-created channels and batches.
- [ ] Run focused tests.

### Task 9: Public start and apply units

**Files:**
- Create: `crates/core/src/start.rs`
- Create: `crates/core/src/apply.rs`
- Create: `crates/core/src/shutdown.rs`
- Create: `crates/core/README.md`
- Modify: `crates/core/src/types.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: core configuration, consumer event/snapshot/wrapper types, and assembled runtime processes.
- Produces: `ConTime::start`, synchronous `ConTime::apply`, memory inspection, and shutdown.

- [ ] Write unit tests for configuration rejection, memory-full batch rejection, successful replay completion, duplicate no-op behavior, and shutdown.
- [ ] Run focused tests and verify the public API is absent.
- [ ] Implement the smallest generic `ConTime` facade and map API/runtime errors without hiding rejected event IDs.
- [ ] Add isolated inline benchmarks for start-side construction and apply-side preparation with downstream work stubbed; do not add an end-to-end benchmark.
- [ ] Update README with contracts and benchmark commands, without recording numbers before benchmarks run.
- [ ] Run core tests and all-target compilation.

### Task 10: Focused verification

**Files:**
- Verify only touched subcrates and uncommitted documentation.

**Interfaces:**
- Consumes: the completed apply pipeline.
- Produces: fresh evidence for correctness and compilation.

- [ ] Run `cargo fmt` for worker, runtime, and core.
- [ ] Run focused tests for worker, runtime, and core.
- [ ] Run `cargo check --all-targets` for worker, runtime, and core.
- [ ] Run every new ignored inline core benchmark in release mode and record the measurements for review.
- [ ] Run `git diff --check` and confirm no commit was created.
