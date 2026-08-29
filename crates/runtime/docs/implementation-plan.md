# ConTime Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an isolated apply-only runtime that starts configurable router and worker threads, connects them through channels, and reports complete shutdown outcomes.

**Architecture:** The crate declares local generic `Router` and `Worker` traits and treats every message as opaque. Every router and worker has a private Crossbeam receiver, and routers receive senders for the complete worker set. A generic `Runtime` owns an indexed router-sender collection and join handles while remaining independent of every other ConTime crate.

**Tech Stack:** Rust 2021, standard threads, `crossbeam-channel` 0.5, inline unit tests, Criterion 0.5 for isolated warm-throughput benchmarks.

**Spec:** `crates/runtime/docs/design.md`

## Global Constraints

- The crate must not depend on `contime` or any ConTime subcrate.
- The first version supports apply execution only.
- The runtime must never inspect, clone, route, apply, replay, or interpret messages.
- Router and worker behavior must be expressed only through traits declared in this crate.
- Router and worker counts must both be nonzero.
- The caller selects a router through the runtime's stable indexed sender collection.
- Each worker owns one private input receiver.
- Factories receive stable zero-based router or worker indexes.
- Shutdown must join every thread and must not return early after one failure.
- Queries, time operations, restarts, recovery, health monitoring, and sibling-crate adapters remain out of scope.
- Do not register this isolated crate in the root workspace during this pass.

## File Map

- `crates/runtime/Cargo.toml`: isolated package metadata and dependencies.
- `crates/runtime/.gitignore`: crate-local build artifacts.
- `crates/runtime/src/lib.rs`: public exports only.
- `crates/runtime/src/types.rs`: configuration, traits, stage identity, errors, outcomes, reports, and the `Runtime` state type.
- `crates/runtime/src/start.rs`: validation, channels, factories, spawning, and rollback.
- `crates/runtime/src/runtime.rs`: apply-input sender access and direct send convenience.
- `crates/runtime/src/shutdown.rs`: dependency-ordered channel closure and complete thread joining.
- `crates/runtime/README.md`: scope, contracts, lifecycle, commands, and measured benchmark result.

---

### Task 1: Crate Skeleton and Compile-Time Contracts

**Files:**
- Create: `crates/runtime/Cargo.toml`
- Create: `crates/runtime/.gitignore`
- Create: `crates/runtime/src/lib.rs`
- Create: `crates/runtime/src/types.rs`

**Interfaces:**
- Consumes: `crossbeam_channel::{Receiver, Sender}` and `std::thread::JoinHandle`.
- Produces: `RuntimeConfig`, `Router`, `Worker`, `RuntimeStage`, `StartError`, `ThreadOutcome`, `ShutdownReport`, and generic `Runtime<I, RE, WE>`.

- [ ] **Step 1: Create the isolated package manifest and ignore file**

```toml
[package]
name = "contime-runtime"
version = "0.1.0"
edition = "2021"
autobenches = false
license = "MIT"
description = "Isolated thread and channel orchestration for ConTime"
publish = false

[dependencies]
crossbeam-channel = "0.5"

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }
```

```gitignore
/target
/Cargo.lock
```

- [ ] **Step 2: Write contract tests in `types.rs` before defining the contracts**

Add a `#[cfg(test)]` module that expects:

```rust
#[test]
fn configuration_preserves_topology_counts() {
    let config = RuntimeConfig { router_count: 2, worker_count: 4 };
    assert_eq!(config.router_count, 2);
    assert_eq!(config.worker_count, 4);
}

#[test]
fn shutdown_report_preserves_every_ordered_outcome() {
    let report = ShutdownReport {
        routers: vec![ThreadOutcome::Completed, ThreadOutcome::Failed("router")],
        workers: vec![ThreadOutcome::Panicked, ThreadOutcome::Completed],
    };
    assert_eq!(report.routers.len(), 2);
    assert_eq!(report.workers.len(), 2);
    assert_eq!(report.routers[1], ThreadOutcome::Failed("router"));
}
```

- [ ] **Step 3: Run the focused tests and verify the missing contracts fail to compile**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib types::tests
```

Expected: compilation fails because `RuntimeConfig`, `ShutdownReport`, and `ThreadOutcome` do not exist.

- [ ] **Step 4: Implement the minimal public contracts in `types.rs`**

Use these exact shapes:

```rust
use std::io;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    pub router_count: usize,
    pub worker_count: usize,
}

pub trait Router: Send + 'static {
    type Input: Send + 'static;
    type WorkerInput: Send + 'static;
    type Error: Send + 'static;

    fn run(
        self,
        input: Receiver<Self::Input>,
        workers: Vec<Sender<Self::WorkerInput>>,
    ) -> Result<(), Self::Error>;
}

pub trait Worker: Send + 'static {
    type Input: Send + 'static;
    type Error: Send + 'static;

    fn run(self, input: Receiver<Self::Input>) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeStage {
    Router { index: usize },
    Worker { index: usize },
}

#[derive(Debug)]
pub enum StartError {
    NoRouters,
    NoWorkers,
    ThreadSpawn { stage: RuntimeStage, source: io::Error },
}

#[derive(Debug, Eq, PartialEq)]
pub enum ThreadOutcome<E> {
    Completed,
    Failed(E),
    Panicked,
}

#[derive(Debug, Eq, PartialEq)]
pub struct ShutdownReport<RE, WE> {
    pub routers: Vec<ThreadOutcome<RE>>,
    pub workers: Vec<ThreadOutcome<WE>>,
}

pub struct Runtime<I, RE, WE> {
    pub(crate) inputs: Vec<Sender<I>>,
    pub(crate) routers: Vec<JoinHandle<Result<(), RE>>>,
    pub(crate) workers: Vec<JoinHandle<Result<(), WE>>>,
}
```

Implement `Display` and `std::error::Error` for `StartError`, returning the nested `io::Error` from `source()` only for `ThreadSpawn`.

- [ ] **Step 5: Export the contracts from `lib.rs` and rerun tests**

```rust
//! Apply-only runtime topology independent of ConTime domain crates.

mod runtime;
mod shutdown;
mod start;
mod types;

pub use types::{
    Router, Runtime, RuntimeConfig, RuntimeStage, ShutdownReport, StartError,
    ThreadOutcome, Worker,
};
```

Create empty `runtime.rs`, `shutdown.rs`, and `start.rs` files so the module declarations resolve.

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib types::tests
```

Expected: both contract tests pass.

- [ ] **Step 6: Commit the contract layer**

```bash
git add crates/runtime/Cargo.toml crates/runtime/.gitignore crates/runtime/src
git commit -m "feat(runtime): define isolated execution contracts"
```

---

### Task 2: Runtime Startup and Shared Router Queue

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `crates/runtime/src/start.rs`
- Modify: `crates/runtime/src/types.rs`

**Interfaces:**
- Consumes: `RuntimeConfig`, `Router`, `Worker`, and `Runtime<I, RE, WE>` from Task 1.
- Produces: `Runtime::start(config, router_factory, worker_factory)` through the default `Runtime<(), (), ()>` constructor namespace.

- [ ] **Step 1: Write failing startup tests in `start.rs`**

Define local fakes whose `run` methods record stable indexes and consume channels. Add tests with these assertions:

```rust
#[test]
fn zero_router_count_is_rejected_before_factories_run() {
    let router_calls = AtomicUsize::new(0);
    let worker_calls = AtomicUsize::new(0);
    let result = Runtime::start(
        RuntimeConfig { router_count: 0, worker_count: 1 },
        |_| { router_calls.fetch_add(1, Ordering::SeqCst); TestRouter },
        |_| { worker_calls.fetch_add(1, Ordering::SeqCst); TestWorker },
    );
    assert!(matches!(result, Err(StartError::NoRouters)));
    assert_eq!(router_calls.load(Ordering::SeqCst), 0);
    assert_eq!(worker_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn zero_worker_count_is_rejected_before_factories_run() {
    let result = Runtime::start(
        RuntimeConfig { router_count: 1, worker_count: 0 },
        |_| TestRouter,
        |_| TestWorker,
    );
    assert!(matches!(result, Err(StartError::NoWorkers)));
}
```

Add a successful-start test in which two router factories and four worker factories push their indexes into shared vectors. Assert the returned runtime owns two router input senders, two router handles, and four worker handles. In a private test helper, clear its input senders, drain and join the router handles, then drain and join the worker handles. Finally assert the sorted factory-index vectors contain `[0, 1]` and `[0, 1, 2, 3]`. This keeps Task 2 independent of the public shutdown method introduced in Task 3.

- [ ] **Step 2: Run the startup tests and verify failure**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib start::tests
```

Expected: compilation fails because `Runtime::start` is not defined.

- [ ] **Step 3: Implement a file-local spawning dependency seam**

In `start.rs`, define:

```rust
trait Deps {
    fn spawn<T, F>(&self, name: String, run: F) -> std::io::Result<JoinHandle<T>>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static;
}

struct DefaultDeps;

impl Deps for DefaultDeps {
    fn spawn<T, F>(&self, name: String, run: F) -> std::io::Result<JoinHandle<T>>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        std::thread::Builder::new().name(name).spawn(run)
    }
}
```

Keep `Deps`, `DefaultDeps`, and `start_with_deps` private to this file so unit tests can inject spawn failures without changing the public API.

- [ ] **Step 4: Implement validation, channels, and thread startup**

Provide this public constructor form:

```rust
impl Runtime<(), (), ()> {
    pub fn start<R, W, RF, WF>(
        config: RuntimeConfig,
        router_factory: RF,
        worker_factory: WF,
    ) -> Result<Runtime<R::Input, R::Error, W::Error>, StartError>
    where
        R: Router<WorkerInput = W::Input>,
        W: Worker,
        RF: FnMut(usize) -> R,
        WF: FnMut(usize) -> W,
    {
        start_with_deps(&DefaultDeps, config, router_factory, worker_factory)
    }
}
```

Inside `start_with_deps`:

1. Reject zero counts before invoking either factory.
2. Create `router_count` `unbounded::<R::Input>()` channels.
3. Create `worker_count` `unbounded::<W::Input>()` channels.
4. Spawn workers in index order using names `contime-worker-{index}`.
5. Spawn routers in index order using names `contime-router-{index}`.
6. Give each router its indexed private receiver and `worker_senders.clone()`.
7. Drop the startup-owned receiver collection and worker senders.
8. Return `Runtime { inputs: input_senders, routers, workers }`.

Implement a private rollback helper that drops all remaining senders and receivers, joins routers before workers, and ignores their outcomes because startup is returning the spawn error.

- [ ] **Step 5: Run startup tests and the complete library test suite**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib start::tests
cargo test --manifest-path crates/runtime/Cargo.toml --lib
```

Expected: all tests pass.

- [ ] **Step 6: Commit startup**

```bash
git add crates/runtime/src
git commit -m "feat(runtime): start router and worker topology"
```

---

### Task 3: Running Apply-Input Boundary

**Files:**
- Modify: `crates/runtime/src/lib.rs`
- Modify: `crates/runtime/src/runtime.rs`

**Interfaces:**
- Consumes: `Runtime<I, RE, WE>` created by Task 2.
- Produces: `Runtime::input`, `Runtime::send`, and `RuntimeSendError<I>`.

- [ ] **Step 1: Write failing input-boundary tests in `runtime.rs`**

Use a router fake that receives `u64` values and forwards them to worker zero, plus a worker fake that records values. Add:

```rust
#[test]
fn send_forwards_one_opaque_input_into_the_running_topology() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let runtime = test_runtime(Arc::clone(&received));

    runtime.send(0, 42).unwrap();
    finish_test_runtime(runtime);

    assert_eq!(*received.lock().unwrap(), vec![42]);
}

#[test]
fn input_returns_the_runtime_owned_sender() {
    let runtime = test_runtime(Arc::new(Mutex::new(Vec::new())));
    runtime.input(0).unwrap().send(7).unwrap();
    finish_test_runtime(runtime);
}
```

`finish_test_runtime` is a private test helper that clears the input sender
collection, then drains and joins router handles followed by worker handles. The
public shutdown contract remains entirely in Task 4.

Add a two-router test that sends the integers `0..1_000`. Each router forwards
every input it receives to worker zero. After shutdown, sort the worker's
recorded values and assert they equal `(0..1_000).collect::<Vec<_>>()`. Also
record a per-router receive count and send alternating values through sender
indexes zero and one. Assert each router receives exactly 500 inputs.

- [ ] **Step 2: Run the focused tests and verify failure**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib runtime::tests
```

Expected: compilation fails because `input`, `send`, and `shutdown` are not yet available.

- [ ] **Step 3: Implement the runtime input API**

Define a local error rather than exposing Crossbeam's error type directly:

```rust
#[derive(Debug, Eq, PartialEq)]
pub struct RuntimeSendError<I> {
    pub router_index: usize,
    pub input: I,
}

impl<I, RE, WE> Runtime<I, RE, WE> {
    pub fn inputs(&self) -> &[Sender<I>] {
        &self.inputs
    }

    pub fn send(&self, router_index: usize, input: I) -> Result<(), RuntimeSendError<I>> {
        // Look up the selected sender and return the input if unavailable.
    }
}
```

Implement `Display` and `std::error::Error` for `RuntimeSendError<I>` when `I: Debug`, and export it from `lib.rs`.

- [ ] **Step 4: Add the private test cleanup helper**

Keep the cleanup helper inside `runtime.rs`'s `#[cfg(test)]` module. It must
close input, join every router, and then join every worker. Do not add the
public `shutdown` method here; Task 4 owns that behavior and its outcome tests.

- [ ] **Step 5: Run input-boundary tests**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib runtime::tests
```

Expected: all input-boundary tests pass.

- [ ] **Step 6: Commit the running boundary**

```bash
git add crates/runtime/src
git commit -m "feat(runtime): expose opaque apply input"
```

---

### Task 4: Complete Shutdown Reporting

**Files:**
- Modify: `crates/runtime/src/shutdown.rs`

**Interfaces:**
- Consumes: `Runtime<I, RE, WE>`, `ThreadOutcome<E>`, and `ShutdownReport<RE, WE>`.
- Produces: `Runtime::shutdown(self) -> ShutdownReport<RE, WE>` that joins every thread in stable index order.

- [ ] **Step 1: Write failing shutdown tests**

Add independent router and worker fakes that return errors or panic after their channels close. Cover:

```rust
#[test]
fn shutdown_collects_every_returned_error_without_returning_early() {
    // Two routers return Err(index), two workers return Err(index + 10).
    let runtime = failing_runtime();
    let report = runtime.shutdown();

    assert_eq!(
        report.routers,
        vec![ThreadOutcome::Failed(0), ThreadOutcome::Failed(1)]
    );
    assert_eq!(
        report.workers,
        vec![ThreadOutcome::Failed(10), ThreadOutcome::Failed(11)]
    );
}

#[test]
fn shutdown_distinguishes_router_and_worker_panics() {
    let report = panicking_runtime().shutdown();
    assert_eq!(report.routers, vec![ThreadOutcome::Panicked]);
    assert_eq!(report.workers, vec![ThreadOutcome::Panicked]);
}
```

Also add a test using atomics to prove every router has returned before the final worker join completes after worker-sender closure.

- [ ] **Step 2: Run shutdown tests and verify any incomplete behavior fails**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib shutdown::tests
```

Expected: at least the multi-error or panic test fails until the final join helper is implemented.

- [ ] **Step 3: Implement complete stable joining**

Use one private helper:

```rust
fn join_all<E>(handles: Vec<JoinHandle<Result<(), E>>>) -> Vec<ThreadOutcome<E>> {
    handles
        .into_iter()
        .map(|handle| match handle.join() {
            Ok(Ok(())) => ThreadOutcome::Completed,
            Ok(Err(error)) => ThreadOutcome::Failed(error),
            Err(_) => ThreadOutcome::Panicked,
        })
        .collect()
}
```

The public method must preserve dependency order:

```rust
impl<I, RE, WE> Runtime<I, RE, WE> {
    pub fn shutdown(mut self) -> ShutdownReport<RE, WE> {
        drop(self.input.take());
        let routers = join_all(std::mem::take(&mut self.routers));
        let workers = join_all(std::mem::take(&mut self.workers));
        ShutdownReport { routers, workers }
    }
}
```

- [ ] **Step 4: Run shutdown and full library tests**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib shutdown::tests
cargo test --manifest-path crates/runtime/Cargo.toml --lib
```

Expected: all tests pass and no test hangs.

- [ ] **Step 5: Commit shutdown reporting**

```bash
git add crates/runtime/src/shutdown.rs
git commit -m "feat(runtime): report complete shutdown outcomes"
```

---

### Task 5: Startup Failure Rollback

**Files:**
- Modify: `crates/runtime/src/start.rs`

**Interfaces:**
- Consumes: the private `Deps` seam and startup state from Task 2.
- Produces: deterministic rollback for worker-spawn and router-spawn failures, returned as `StartError::ThreadSpawn` with the exact stage index.

- [ ] **Step 1: Add a file-local failing spawner and rollback tests**

Implement `StubDeps` with an atomic spawn-call counter and a configured failing call. Successful calls delegate to `std::thread::spawn`; the selected call returns `io::Error::other("stub spawn failure")`. Give each fake thread an exit-notification sender, and make assertions with `recv_timeout(Duration::from_secs(1))` so a broken rollback fails instead of hanging the test process.

Add tests that prove:

```rust
#[test]
fn worker_spawn_failure_closes_and_joins_workers_already_started() {
    // Fail while starting worker index 2.
    // Workers 0 and 1 increment an exited counter after receiver closure.
    // Assert StartError identifies Worker { index: 2 } and exited == 2.
}

#[test]
fn router_spawn_failure_closes_and_joins_the_entire_partial_topology() {
    // Start two workers and router 0, then fail router index 1.
    // Assert the error identifies Router { index: 1 }.
    // Assert router 0 and both workers observed disconnection and returned.
}
```

- [ ] **Step 2: Run rollback tests and verify failure**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib start::tests::worker_spawn_failure
cargo test --manifest-path crates/runtime/Cargo.toml --lib start::tests::router_spawn_failure
```

Expected: tests fail because one or more exit notifications do not arrive before the one-second bound, or because the returned stage is incorrect.

- [ ] **Step 3: Implement dependency-ordered rollback**

For worker failure:

1. Drop every router input sender and receiver.
2. Drop every worker sender and every unowned worker receiver.
3. Join all worker handles already created.
4. Return `StartError::ThreadSpawn { stage: RuntimeStage::Worker { index }, source }`.

For router failure:

1. Drop every router input sender and remaining receiver.
2. Drop startup-owned worker senders.
3. Join every started router so their worker-sender clones are dropped.
4. Join every worker.
5. Return `StartError::ThreadSpawn { stage: RuntimeStage::Router { index }, source }`.

Never return from rollback before every previously started handle has been joined.

- [ ] **Step 4: Run rollback and full tests**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib start::tests
cargo test --manifest-path crates/runtime/Cargo.toml --lib
```

Expected: all tests pass and both rollback counters prove complete cleanup.

- [ ] **Step 5: Commit rollback behavior**

```bash
git add crates/runtime/src/start.rs
git commit -m "fix(runtime): clean up partial thread startup"
```

---

### Task 6: Isolated Runtime Benchmark and Documentation

**Files:**
- Modify: `crates/runtime/src/runtime.rs`
- Create: `crates/runtime/README.md`

**Interfaces:**
- Consumes: the complete public runtime API.
- Produces: one external Criterion/pprof integration benchmark and user-facing crate documentation.

- [x] **Step 1: Add an external Criterion benchmark with flamegraphs**

In `benches/runtime.rs`, start each runtime topology before timing. Criterion's
untimed setup creates one completion channel and clones its sender into every
prepared input. Each timed iteration must:

1. Send 1,000 benchmark events through their encoded router index.
2. Forward each event through its encoded worker index.
3. Wait for the prepared receiver to close after workers drop every forwarded
   sender clone.
4. Leave the runtime alive for the next iteration.

Register topology benchmarks for one router/one worker and two routers/four workers.

The benchmark indexes the runtime's router-sender slice directly. It performs
no hashing, modulo, runtime trait lookup, or acknowledgement messages. Shut
down only after Criterion finishes. The benchmark must not import sibling
crates.

- [ ] **Step 2: Run tests and the benchmark**

Run:

```bash
cargo test --manifest-path crates/runtime/Cargo.toml --lib
cargo bench --manifest-path crates/runtime/Cargo.toml --bench runtime
```

Expected: all unit tests pass and Criterion prints a stable median for the named workload.

- [ ] **Step 3: Write `README.md` with the measured result**

Document:

- isolated purpose and exclusions;
- local `Router` and `Worker` contracts;
- private indexed router and worker queues;
- configuration and factory indexing;
- explicit shutdown and external-sender lifetime rule;
- startup rollback and lack of automatic recovery;
- exact test and benchmark commands; and
- the measured median, nanoseconds per batch, batches per second, and derived logical events per second.

State clearly that startup and shutdown are outside timing. The measurement includes warm channel traffic, no-op routing, no-op worker processing, and completion observation. It excludes real routing, event storage, replay, lanes, and API rejection collection.

- [ ] **Step 4: Run final focused verification**

Run:

```bash
cargo fmt --manifest-path crates/runtime/Cargo.toml -- --check
cargo test --manifest-path crates/runtime/Cargo.toml
cargo clippy --manifest-path crates/runtime/Cargo.toml --all-targets -- -D warnings
```

Expected: formatting is clean, all tests pass, and Clippy emits no warnings.

- [ ] **Step 5: Audit the crate boundary**

Run:

```bash
rg -n "contime-(api|router|worker|events|checkpoints|lanes)|path =" crates/runtime/Cargo.toml crates/runtime/src
```

Expected: no matches. Confirm `git diff -- crates/runtime` contains no changes outside the isolated crate.

- [ ] **Step 6: Commit the completed runtime crate**

```bash
git add crates/runtime
git commit -m "feat: add isolated runtime crate"
```
