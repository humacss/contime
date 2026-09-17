# Minimal Prepared Apply Pipeline Report

## Scope

- The API consumes inputs into one `AHashMap` from snapshot ID to the final owned snapshot batch.
- A one-route input is moved without cloning; an input with `N` routes is cloned exactly `N - 1` times.
- The router hashes each snapshot ID once and initializes messages only for affected workers.
- Workers receive complete `(snapshot_id, batch)` pairs and apply them directly to snapshot histories in a plain blocking receive loop.
- Successful input completion is final-sender disconnection. Workers transmit only non-empty rejection vectors.
- API, router, worker, and snapshot-history benchmarks use one real warm-up event before measuring 1 or 1,000 later events.

## RED evidence

| Focused test | Captured pre-implementation failure |
| --- | --- |
| `cargo test --test snapshot_batching -- --nocapture` | Compilation failed because `SnapshotBatchBenchmark::group` returned `Vec<(u128, Vec<u128>)>` and therefore could not be queried by snapshot ID. |
| `cargo test router::tests --lib -- --nocapture` | Compilation failed because `Router::dispatch_prepared_request` did not exist; the router accepted only the old batch vector. |
| `cargo test --test apply_context send_returns_after_enqueue_and_success_is_completion_by_disconnect -- --nocapture` | The assertion received `Ok([])` instead of `Err(Disconnected)`, proving successful workers still transmitted empty responses. |
| `cargo test --test apply_boundary_benchmarks benchmark_adapters_apply_a_real_warm_input_before_measured_work -- --nocapture` | Compilation failed because the router and worker benchmark adapters had no `apply_inputs` method for a real warm-up event. |

## GREEN evidence

| Verification command | Captured result |
| --- | --- |
| `cargo test --test snapshot_batching` | 5 passed, 0 failed. |
| `cargo test --test apply_context` | 12 passed, 0 failed. |
| `cargo test --test router_api_boundary` | 2 passed, 0 failed. |
| `cargo test --test apply_boundary_benchmarks` | 6 passed, 0 failed. |
| `cargo test --test memory` | 16 passed, 0 failed. |
| `cargo test --test router_allocations` | 2 passed, 0 failed; the 1,000-event one-worker partition reported 2 allocations in the focused run. |
| `cargo fmt --check` | Passed with no formatting differences. |
| `cargo clippy --all-targets -- -D warnings` | Passed with 0 warnings and 0 errors. |
| `cargo test --all-targets` | 148 Rust tests, 68 Criterion smoke scenarios, 3 trybuild compile-fail fixtures, and the example target passed; 0 failures. |
| `cargo bench --bench apply_boundaries -- apply_boundaries --sample-size 30` | All 8 warmed boundary measurements completed. |
| `cargo bench --bench router -- completion_by_disconnect --sample-size 30` | All 3 sender-drop completion measurements completed. |

## Benchmarks

All intervals are Criterion's exact `[low estimate high]` results from the optimized profile on 2026-08-28. The per-event value divides the 1,000-input point estimate by 1,000.

| Boundary | One-input interval | 1,000-input interval | 1,000-input per-event point estimate |
| --- | ---: | ---: | ---: |
| Public API | `[12.532 µs 12.802 µs 13.089 µs]` | `[90.242 µs 101.20 µs 113.71 µs]` | `101.20 ns` |
| Router | `[12.127 µs 12.539 µs 13.134 µs]` | `[57.554 µs 58.159 µs 58.914 µs]` | `58.159 ns` |
| Worker | `[12.216 µs 12.719 µs 13.258 µs]` | `[56.651 µs 57.514 µs 58.506 µs]` | `57.514 ns` |
| Snapshot history | `[198.63 ns 275.74 ns 347.26 ns]` | `[40.823 µs 41.398 µs 42.325 µs]` | `41.398 ns` |

| Completion senders | Completion-by-disconnection interval |
| ---: | ---: |
| 1 | `[99.193 ns 99.458 ns 99.740 ns]` |
| 2 | `[101.38 ns 102.13 ns 103.30 ns]` |
| 8 | `[120.90 ns 121.24 ns 121.62 ns]` |

## Deferred work

- Cross-worker transactional memory admission and rollback remain deferred; advisory API checks can still be followed by partial worker application under contention.
- Timeless Runtime benchmarks must be updated after this ConTime API change.
- Spacetime runtime benchmarks must be rerun after its ConTime dependency adopts the new API.
