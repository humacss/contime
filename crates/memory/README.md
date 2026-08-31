# ConTime Memory

`contime-memory` provides isolated tracked replacements for `Arc` and `Box`.
It depends on neither the root ConTime crate nor another ConTime subcrate.

## Contract

The crate exposes one value and three consumer-defined traits:

- `SizeDelta` represents an increase, decrease, or unchanged tracked size.
- `ConservativeTrackedSize` measures an underlying value and the memory it
  retains.
- `TrackedSizeDelta` runs a mutation on its implementing value and returns the
  mutation result together with its `SizeDelta`.
- `TrackedMemoryBudget` consumes deltas and exposes the configured safety
  buffer.

The memory crate provides no concrete budget or sizing implementation. Core is
free to implement accounting with atomics, channels, local counters, or another
mechanism without changing the tracked ownership types.

## Tracked ownership

`TrackedArc<T, B>` owns one immutable shared allocation. Creation reports the
allocation and first handle, cloning reports another handle, every handle drop
releases that handle, and the final drop releases the shared allocation exactly
once.

`TrackedBox<T, B>` owns one independently mutable allocation. Cloning deeply
clones `T` and reports a new allocation. Mutable access exists only through
`update`, which delegates the action to `T::size_delta` and applies its returned
delta to the budget.

Both public wrappers are exactly one machine pointer wide. Their budget handles
live beside their values in the corresponding heap allocations. Neither wrapper
exposes `DerefMut` or `into_inner`.

## Verification

```sh
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo test --manifest-path crates/memory/Cargo.toml
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo check --manifest-path crates/memory/Cargo.toml --all-targets
```

The two diagnostic unit benchmarks are ignored tests:

```sh
CARGO_TARGET_DIR=/private/tmp/contime-memory-target cargo test \
  --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_tracked_arc -- --ignored --nocapture --test-threads=1

CARGO_TARGET_DIR=/private/tmp/contime-memory-target cargo test \
  --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_tracked_box -- --ignored --nocapture --test-threads=1
```

The public lifecycle benchmark is a normal Criterion integration target:

```sh
CARGO_TARGET_DIR=/private/tmp/contime-memory-target \
  cargo bench --manifest-path crates/memory/Cargo.toml --bench ownership
```

## Benchmark results

Measured on the current development machine in release mode with Criterion.
Every integration iteration executes exactly 1,000 named operations. The median
is the center estimate from the most recent run.

### Budget comparison with 64-byte values

`Difference` is tracked median minus the matched standard-library median.

| Lifecycle | Budget | Standard | Tracked | Difference | Overhead | Tracked throughput |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Arc create/drop | No-op | 25.639 ns | 26.227 ns | +0.588 ns | +2.29% | 38.129M/s |
| Arc create/drop | Local | 25.639 ns | 27.255 ns | +1.616 ns | +6.30% | 36.690M/s |
| Arc create/drop | Atomic | 25.639 ns | 25.825 ns | +0.186 ns | +0.73% | 38.723M/s |
| Arc clone/drop | No-op | 4.7545 ns | 4.7603 ns | +0.0058 ns | +0.12% | 210.07M/s |
| Arc clone/drop | Local | 4.7545 ns | 4.7734 ns | +0.0189 ns | +0.40% | 209.50M/s |
| Arc clone/drop | Atomic | 4.7545 ns | 4.7929 ns | +0.0384 ns | +0.81% | 208.64M/s |
| Box create/drop | No-op | 53.155 ns | 49.192 ns | -3.963 ns† | -7.46%† | 20.328M/s |
| Box create/drop | Local | 53.155 ns | 55.643 ns | +2.488 ns | +4.68% | 17.972M/s |
| Box create/drop | Atomic | 53.155 ns | 54.151 ns | +0.996 ns | +1.87% | 18.467M/s |
| Box update | No-op | 1.0122 ns | 0.7586 ns | -0.2536 ns† | -25.05%† | 1.3182B/s |
| Box update | Local | 1.0122 ns | 0.6896 ns | -0.3226 ns† | -31.87%† | 1.4502B/s |
| Box update | Atomic | 1.0122 ns | 2.4250 ns | +1.4128 ns | +139.58% | 412.37M/s |
| Box deep-clone/drop | No-op | 51.482 ns | 51.592 ns | +0.110 ns | +0.21% | 19.383M/s |
| Box deep-clone/drop | Local | 51.482 ns | 56.777 ns | +5.295 ns | +10.29% | 17.613M/s |
| Box deep-clone/drop | Atomic | 51.482 ns | 56.433 ns | +4.951 ns | +9.62% | 17.720M/s |

### Deep Box clone/drop by retained payload

| Payload | Budget | Standard | Tracked | Difference | Overhead | Tracked throughput |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 64 bytes | No-op | 51.482 ns | 51.592 ns | +0.110 ns | +0.21% | 19.383M/s |
| 64 bytes | Local | 51.482 ns | 56.777 ns | +5.295 ns | +10.29% | 17.613M/s |
| 64 bytes | Atomic | 51.482 ns | 56.433 ns | +4.951 ns | +9.62% | 17.720M/s |
| 256 bytes | No-op | 58.270 ns | 58.191 ns | -0.079 ns† | -0.14%† | 17.185M/s |
| 256 bytes | Local | 58.270 ns | 59.842 ns | +1.572 ns | +2.70% | 16.711M/s |
| 256 bytes | Atomic | 58.270 ns | 59.483 ns | +1.213 ns | +2.08% | 16.811M/s |
| 1,024 bytes | No-op | 72.678 ns | 71.363 ns | -1.315 ns† | -1.81%† | 14.013M/s |
| 1,024 bytes | Local | 72.678 ns | 76.391 ns | +3.713 ns | +5.11% | 13.091M/s |
| 1,024 bytes | Atomic | 72.678 ns | 79.009 ns | +6.331 ns | +8.71% | 12.657M/s |

Each standard baseline performs the same Rust ownership and payload work as its
tracked counterpart. Arc creation performs one heap allocation. Box creation
and deep cloning each perform two: one for the retained `Vec` payload and one
for the Box. Box update operates on 1,000 persistent Boxes, performs no
allocation, and mutates the same payload byte. Arc clone/drop measures a
complete shared handle lifetime.

The no-op budget consumes each delta through `black_box` but stores nothing, so
its difference is the closest estimate of wrapper and delta-construction cost.
The local budget uses `Rc<Cell<isize>>`; the atomic budget uses
`Arc<AtomicIsize>` with relaxed ordering.

The differences are derived by subtracting independently sampled medians; they
are not directly timed differential measurements. Values marked † have a lower
tracked median. They demonstrate compiler, code-layout, or allocator effects
and must not be interpreted as negative overhead. Very small positive values
may likewise be below reliable measurement resolution. The clearest hot-path
cost is atomic Box reporting: approximately 1.413 ns beyond direct mutation.
Arc clone/drop adds at most 0.038 ns in this run, while allocation-heavy paths
are dominated by allocator and payload work.

These integration measurements exclude API, router, worker, events,
checkpoints, lanes, runtime orchestration, and any concrete core memory policy.
