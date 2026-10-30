# Workspace and verification

[Overview](../../README.md) · [Using ConTime](../usage/README.md) · [How it works](../how-it-works/README.md)

The root crate is a facade: it re-exports application contracts and the selected
execution model. Most implementation lives in the subcrates below. Runtime starts
managed background processing; Manual runs an existing worker when the host calls
`process()`. Core supplies their shared application contracts and storage adapters.

## Shared source and publication

Only `contime` is published to crates.io. Its root library compiles the component
implementations directly as modules and re-exports the consumer-facing API.
Cargo does not bundle workspace dependencies into a published package, so the
root package has no internal `contime-*` package dependencies.

Components keep conventional, non-publishable development packages under
`crates/<component>/Cargo.toml`, with implementation, tests and benchmarks beside
them. The root facade includes each component through `src/_symlinks/`, a set
of tracked symbolic links to that same source. Each component's `src/lib.rs` is
its standalone entry point; `src/mod.rs` is its embedded entry point. Both stay
thin and declare the same implementation files.

Cargo excludes nested packages from the parent archive, but copies symlink
targets into the archive as ordinary files. This lets the root package contain
the shared implementation without the component manifests. There is one
maintained implementation; the links need no regeneration or release script.
Workspace commands such as `cargo test -p contime-worker` and
`cargo bench -p contime-batcher --bench batcher` continue to select individual
components. The [symlink guide](../../src/_symlinks/README.md) explains
the source links, relative fixtures, and symlink requirements for development checkouts.
Published-package consumers receive ordinary files.

CI checks the link targets and archive contents, then extracts the package
outside the checkout and runs its tests and examples. Ordinary component
manifests and original source directories are absent from that extracted tree.


Application code should use types re-exported by `contime` consistently. A type
compiled inside `contime` and the same source compiled in a standalone development
crate are distinct Rust types and cannot be mixed interchangeably.

Verify the one public package without uploading it:

```sh
cargo publish -p contime --all-features --dry-run
```

## Crates

| Crate | Responsibility |
| --- | --- |
| [`contime-monitor`](../../crates/monitor/README.md) | Read-only worker status, workload accounting, and bounded observation history |
| [`contime-simulator`](../../crates/simulator/README.md) | Caller-driven generation, independent validation, and paced Maximum admission |
| [`contime-batcher`](../../crates/batcher/README.md) | Bounded packetization and ready-queue draining for the native pipeline |
| [`contime-manual`](../../crates/manual/README.md) | Caller-driven processing of one worker between complete operations |
| `contime` (root) | Public facade and selection of manual or managed runtime processing |
| [`contime-core`](https://github.com/humacss/contime/blob/main/crates/core/README.md) | Shared application contracts and synchronous history/storage adapters |
| [`contime-api`](https://github.com/humacss/contime/blob/main/crates/api/README.md) | Input requests, responses, and optional local callback adapters |
| [`contime-router`](https://github.com/humacss/contime/blob/main/crates/router/README.md) | Deterministic routing to worker-owned histories |
| [`contime-runtime`](https://github.com/humacss/contime/blob/main/crates/runtime/README.md) | Managed instance, admission, batching integration, router/worker threads, and shutdown |
| [`contime-worker`](https://github.com/humacss/contime/blob/main/crates/worker/README.md) | History ownership and incremental replay scheduling |
| [`contime-snapshots`](https://github.com/humacss/contime/blob/main/crates/snapshots/README.md) | Checkpoint storage and historical reconstruction |
| [`contime-events`](https://github.com/humacss/contime/blob/main/crates/events/README.md) | Ordered retained event history |
| [`contime-lanes`](https://github.com/humacss/contime/blob/main/crates/lanes/README.md) | Consumer application and hook contracts |
| [`contime-progress`](https://github.com/humacss/contime/blob/main/crates/progress/README.md) | Participant progress measurement |

`contime-manual` owns caller-driven processing; `contime-batcher` owns transport batching for the native pipeline. Neither depends on the other.

Verify each concern independently:

```bash
cargo +1.90.0 test -p contime-manual
cargo +1.90.0 test -p contime-batcher
cargo +1.90.0 bench -p contime-batcher --bench batcher
cargo +1.90.0 test --manifest-path examples/environments/wasi/Cargo.toml
```

The [WebAssembly example](../../examples/environments/wasi/README.md) verifies Manual and threaded Runtime
execution through their respective WASI targets; the [browser example](../../examples/environments/browser/README.md) adds a
configurable browser host.

## Current constraints

- ConTime is in-memory only. Persistence, replay from disk, and recovery belong in surrounding systems.
- Retention prunes old events and checkpoints by time. There is no built-in memory cap.
- Application logic must be deterministic. Stable event IDs and snapshot routing belong to the consumer.
- Queries earlier than retained history cannot reconstruct discarded events.
- Checkpoints clone snapshot state, so large snapshots can be expensive.
- Submission and advancement across workers do not provide transactional rollback. Integrations that need stronger ordering must supply it at their own boundary.
- The API is still evolving. The root re-exports the current input and application contracts; application-specific macros belong in consumers.

## Verification

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p contime --example readme
cargo run -p contime --all-features --example client_movement -- --headless --duration 2
cargo test --doc --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Build the Rust API documentation with `cargo doc --workspace --no-deps --open`
for completion boundaries and public contracts. Each crate README describes
its focused tests and benchmarks. Recorded component results stay in those
READMEs; public pipeline and terminal-presentation results are in
[`benches/README.md`](../../benches/README.md).

### Integration suites

Root integration tests are grouped by public feature. Each folder has a `main.rs`
that declares topic modules; the individual test files remain separate. Shared
fixtures live under `tests/support/`. Component-specific tests and unit tests stay
in their development crates.

| Suite | Folder | Coverage |
| --- | --- | --- |
| `runtime` | `tests/runtime/` | Admission, queries, listening, progress, batching and replay |
| `manual` | `tests/manual/` | Caller-driven processing and callbacks |
| `monitor` | `tests/monitor/` | Prefix accounting, worker collection and activity reporting |
| `simulator` | `tests/simulator/` | Event generation, late replay and adaptive monitoring feedback |

Run a suite or select a topic within it:

```sh
cargo test -p contime --test runtime
cargo test -p contime --test runtime query::
cargo test -p contime --no-default-features --features manual --test manual
cargo test -p contime --features monitor --test monitor
cargo test -p contime --features simulator --test simulator
cargo test -p contime --all-features --tests
```

Monitor and Simulator also run without an execution backend. Their Runtime and
Manual integration modules run only when the corresponding backend is enabled;
adaptive simulation checks additionally require `monitor`. Neither suite enables
additional features automatically. For independent tool checks:

```sh
cargo test -p contime --no-default-features --features monitor --test monitor
cargo test -p contime --no-default-features --features simulator --test simulator
```

The [coverage workflow](../../.github/workflows/coverage.yml) combines workspace tests
with default and optional features, independent Manual/Monitor/Simulator suites, and example verification
tests. It reports library coverage to [Qlty](https://qlty.sh/gh/humacss/projects/contime),
excluding test, benchmark, and example source. Browser and WASI packages are
separate workspaces and are not included in this native coverage report.
