# Router and API Boundary Implementation Report

Date: 2026-08-26

## Outcome

ConTime now has the approved ownership boundary:

- `Router` visits routes, partitions request batches, dispatches at most one
  message per affected worker, and returns the affected-worker count.
- The production router never receives or waits and owns no input identity,
  horizon admission, memory admission, public outcome, current-time, query
  merge, or inspection merge state.
- `send` creates no response channel and returns after enqueue.
- Every synchronous API call owns a distinct request-scoped channel. Workers
  reply directly to that channel exactly once per worker batch.
- `apply` returns a sorted, deduplicated `Vec<EventRejection>`; rejections
  contain only event ID and reason.
- Workers own retained identity, horizon rejection, memory reservation, and
  partial admission.
- Generated route extraction uses a visitor instead of allocating a
  snapshot-ID vector per input.

Timeless Runtime was not modified.

## Commits

- `17947cc` — direct event-rejection result model
- `9e24b8e` — allocation-free snapshot-route visitor
- `ee93485` — request-scoped worker completion mode
- `d782537` — pure input dispatch and worker-local admission
- `0f32708` — API-owned query, inspection, and advancement aggregation
- `6f35bd2` — isolated partition/allocation/latency benchmarks

The documentation and this report are committed together after the final gate;
the containing commit is intentionally not self-referential.

## RED/GREEN Evidence

1. Rejection model RED: `cargo test api::tests::rejection_merge -- --nocapture`
   failed because `EventRejection`, `EventRejectionReason`, and
   `merge_event_rejections` did not exist. GREEN: both API unit tests passed and
   every integration target compiled.
2. Route visitor RED:
   `cargo test derived_event_route_initializes_only_snapshot_identity -- --nocapture`
   failed because neither routing trait exposed `visit_snapshot_ids`. GREEN:
   exact derive, fragment, marker, empty-route, history-count, and UI test
   binaries passed.
3. Worker completion RED:
   `cargo test worker::tests::responding_completion_sends_exactly_one_batch_result -- --nocapture`
   failed because `Completion` and the completion helper did not exist. GREEN:
   the exact-one-response test and all 12 apply-context tests passed, including
   the 1,000-input single worker-batch fixture.
4. Pure dispatch RED:
   `cargo test router::tests::dispatch_inputs_reports_only_affected_workers -- --nocapture`
   failed because `dispatch_inputs` did not exist. GREEN: two-of-eight affected
   dispatch, concurrent request isolation, multi-worker rejection deduplication,
   memory rejection, identity, horizon, journal, edge, and generic-time tests
   passed.
5. Remaining operation dispatch RED:
   `cargo test router::tests::query_dispatch_returns_one_affected_worker -- --nocapture`
   failed because dispatch-only query, inspection, and advancement methods did
   not exist. GREEN: all dispatch tests and the full public query, journal,
   memory, and apply-context suites passed.
6. Allocation RED:
   `cargo test --test router_allocations -- --nocapture --test-threads=1`
   failed because no production partition benchmark adapter existed. GREEN: the
   one-worker 1,000-event partition uses exactly 3 allocations.

## Isolated Performance Evidence

Environment: Apple M3 Pro, macOS 26.3.1 (25D771280a), rustc 1.90.0
(`1159e78c4 2025-09-14`), optimized benchmark profile, 30 Criterion samples,
2026-08-26. Intervals are exact `[low estimate high]` outputs.

| Boundary | Shape | Count | Interval |
| --- | --- | ---: | ---: |
| Partition | Single target, one worker | 1 | `[103.35 ns 103.67 ns 104.03 ns]` |
| Partition | Single target, one worker | 100 | `[680.37 ns 681.59 ns 682.67 ns]` |
| Partition | Single target, one worker | 1,000 | `[6.8081 µs 6.8701 µs 6.9583 µs]` |
| Partition | Single target, eight workers | 1 | `[293.70 ns 294.71 ns 295.60 ns]` |
| Partition | Single target, eight workers | 100 | `[1.2563 µs 1.2616 µs 1.2653 µs]` |
| Partition | Single target, eight workers | 1,000 | `[7.8877 µs 7.9062 µs 7.9229 µs]` |
| Partition | Three targets, eight workers | 1 | `[307.23 ns 308.01 ns 308.60 ns]` |
| Partition | Three targets, eight workers | 100 | `[3.3050 µs 3.3095 µs 3.3136 µs]` |
| Partition | Three targets, eight workers | 1,000 | `[32.076 µs 32.149 µs 32.215 µs]` |
| Enqueue | One worker message | 1 | `[9.5300 ns 9.6141 ns 9.6736 ns]` |
| Enqueue | One worker message | 100 | `[10.142 ns 10.834 ns 11.705 ns]` |
| Enqueue | One worker message | 1,000 | `[9.3010 ns 9.5532 ns 9.8233 ns]` |
| API completion | Empty rejections | 1 worker | `[149.61 ns 150.07 ns 150.61 ns]` |
| API completion | Empty rejections | 2 workers | `[159.98 ns 160.75 ns 161.34 ns]` |
| API completion | Empty rejections | 8 workers | `[241.07 ns 241.64 ns 242.24 ns]` |

Commands:

```bash
cargo test --test router_allocations -- --nocapture --test-threads=1
cargo bench --bench router -- router_partition --sample-size 30
cargo bench --bench router -- router_enqueue --sample-size 30
cargo bench --bench router -- api_completion --sample-size 30
```

Partition timings exclude fixture construction. Enqueue timings include only
one send of an already-built vector and exclude partitioning, replay, and
consumer-side destruction. API completion includes request-channel creation,
the named empty responses, receive, and production rejection aggregation; it
excludes routing, worker scheduling, admission, history, and replay.

## Final Verification

All final commands passed:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --doc
cargo bench --bench apply --no-run
cargo bench --bench router --no-run
```

Counts from `cargo test --all-targets`:

- 34 library unit tests;
- 88 integration tests across 18 integration binaries;
- 2 compile-fail fixtures inside the UI integration test;
- 87 Criterion smoke cases across the `apply`, `channels`, `flume`, `query`,
  and `router` benchmark targets;
- the compiled example test target.

`cargo test --doc` passed 1 doctest. Both requested optimized benchmark targets
compiled successfully.

The final mechanical scan was:

```bash
rg "\.recv\(" src/router.rs
rg "ApplyOutcome|InputRejection|accepted_input_ids|canonical_inputs|snapshot_ids\(\) -> Vec" \
  src/router.rs src/api.rs src/worker.rs src/traits.rs contime_macros/src/lib.rs
```

Both searches returned no matches. Expected receives remain in `api.rs` for
request aggregation and in `worker.rs` for the worker event loop and its unit
test. `git diff --check` passed before the documentation commit.

## Deferred Work

- Worker journal data structures and worker-side input regrouping.
- History bulk admission and checkpoint/replay optimization.
- Snapshot hash caching.
- Stronger memory-reservation estimates for replay/checkpoint growth.
- Cross-worker transactional ordering; partial dispatch/admission remains an
  explicit property.
- Timeless Runtime performance investigation, which remains outside this
  repository and this change.
