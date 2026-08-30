# ConTime Memory

`contime-memory` is an isolated retained-memory accounting crate. It has no
normal dependencies on ConTime or its other subcrates.

## Model

Memory is accounted in two categories:

- `Allocation` is the conservative retained size of a shared value and its
  allocation metadata. It is reserved once and released when the final strong
  pointer is dropped.
- `Pointer` is the size of one `TrackedArc` handle. It is reserved for every
  live handle and released whenever that handle is dropped.

Values implement `ConservativeSize`. `TrackedArc::try_new` reserves the shared
allocation before reserving its first pointer, and rolls the allocation
reservation back if the pointer reservation fails. `TrackedArc` deliberately
does not implement `Clone`; callers use the fallible `try_clone` operation so a
new pointer can never bypass the configured memory limit.

`MemoryBudget` uses shared atomic counters. Pointer releases may therefore
happen concurrently, while the inner allocation's `Drop` releases the shared
allocation exactly once.

## Benchmarks

Measured on the current development machine in release mode with inline
Criterion benchmarks:

| Operation | Median |
| --- | ---: |
| Reserve and release one pointer charge | 10.57 ns |
| Construct a new tracked allocation | 16.29 ns |
| Fallibly clone a tracked pointer | 5.72 ns |
| Drop a non-final pointer | 4.50 ns |
| Drop the final pointer and allocation | 29.35 ns |

The construction benchmark includes budget creation, allocation and initial
accounting. The clone benchmark includes pointer reservation and `Arc` cloning.
The non-final drop releases one pointer charge; the final drop also destroys the
shared allocation and releases its allocation charge. These are unit-level
measurements only: they exclude API, router, worker, events, checkpoints and
lanes work.

Run the tests and individual benchmarks with:

```sh
cargo test --manifest-path crates/memory/Cargo.toml
cargo test --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_budget -- --ignored --nocapture --test-threads=1
cargo test --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_try_new -- --ignored --nocapture --test-threads=1
cargo test --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_try_clone -- --ignored --nocapture --test-threads=1
cargo test --manifest-path crates/memory/Cargo.toml --release --lib \
  benchmark_drop -- --ignored --nocapture --test-threads=1
```
