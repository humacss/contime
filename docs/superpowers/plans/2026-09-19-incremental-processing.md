# Incremental Processing Implementation Plan

> Execute inline, with test-first verification at each boundary. Do not commit existing or new changes without user instruction.

**Goal:** Implement asynchronous admission, timestamp-ordered interruptible replay, and conservative coordinated pruning in ConTime, without changing the separate `timeless-4d-runtime` repository. ConTime's own `crates/runtime` threading helper remains in scope.

**Architecture:** An admission coordinator owns external/internal cutoffs and measurement rounds. Routers retain transport batching and configurable modulus placement. Workers own timestamp scheduling and resumable checkpoint progress, servicing messages between timestamp buckets.

**Tech stack:** Rust, existing Crossbeam channels, existing isolated ConTime crates; no mutexes or additional channel dependencies.

**Spec:** Approved inline design in this task, September 19.

## Constraints

- Preserve existing memory-budget changes in core/src/memory.rs and core/src/start.rs.
- Queries reconstruct a clone from valid checkpoints and admitted history, without callbacks or replay barriers.
- Advanced time bounds runnable work. Future events remain retained but do not prevent idle.
- Internal outputs must not precede their source timestamp and must enter coordinator transport before worker reports.
- All accepted routes survive admission-cutoff changes. Prune strictly before a proven boundary, retaining its anchor.
- Missing round participants never authorize pruning.

## Execution order

- [x] Checkpoint crate: add tests for one-bucket progress, late same-time insertion, invalidated suffixes, and query reconstruction during partial replay. Use the owned valid tip as the continuation cursor; move partial tips without cloning, preserve checkpoint counts, and leave acknowledgement to the scheduling caller.
- [x] Worker crate: replace whole-history replay scheduling with earliest timestamp scheduling; drain messages before each calculation step; retain transport batches. Test timestamp order across snapshots, bounded advancement, and query priority.
- [x] Router placement: configurable modulus default consistently across queries/listeners/applies; test fanout and custom placement. Core configuration and its tests/benchmarks use `placement` instead of `router_seed`.
- [x] Router coordination: add round marker dispatch after active routing work.
- [x] Core: add admission coordinator and asynchronous apply/shared errors. Separate processing target, requested retention cutoff, and proven cutoff. Preserve accepted routes.
- [x] Coordination: serialize round opening with admissions, flush pre-round router work through per-router worker markers, collect worker minima, and cap publication by submissions during the round. Test delayed routers, post-report internal work, fanout, no-work workers, and missing participants.
- [x] Integration: update core tests to explicit advance/idle semantics; test live internal submission and deferred pruning. Run focused crate tests, formatting, and clippy; report API breaks affecting Runtime without editing it.

## Verification commands

Use Rust 1.95.0, offline, with one shared target directory. Run `cargo test --manifest-path crates/<crate>/Cargo.toml` per touched crate, first the new failing test and then its complete suite. Use deterministic channels/barriers to control interleavings; do not rely on sleeps for race correctness. Run optimized existing benchmarks only after behavior is verified.

## Current implementation status

The isolated `crates/core` pipeline now uses incremental worker scheduling,
asynchronous admission, and FIFO-fenced safe pruning. The legacy root crate is
unchanged; neither it nor the separate Runtime/game consumers are migrated.

Verified: checkpoints 26 tests, worker 35 tests, router 28 tests, core 38 unit
tests and 19 integration tests, and the ConTime threading helper's 15 tests.
Core benchmark smoke checks passed all 58 cases; no new optimized timing
claims are made. Core all-target Clippy passes with warnings denied. Formatting
and diff whitespace checks pass. Worker and coordinator reviews approved the
pending implementation after adding delayed-router and live-callback coverage.

Consumer changes for a later pass: configure `placement` rather than
`router_seed`; `apply` returns enqueue success, rejections come from `errors()`;
explicitly advance the processing target and use idle waits where completion is
required. Causal callbacks use `apply_internal` before returning. Legacy `send`
and `send_advance_to` remain low-level delivery APIs, not replay barriers.
All changes remain uncommitted for user review.
