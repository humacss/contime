# Native scaling and latency benchmarks

This standalone, unpublished benchmark package measures native worker scaling
and end-to-end application/query latency. It is separate from the browser
example, and is excluded from the workspace and published packages.

Run commands from the ConTime repository root:

```sh
cargo +1.90.0 test --manifest-path benches/scaling/Cargo.toml
cargo +1.90.0 clippy --manifest-path benches/scaling/Cargo.toml --all-targets -- -D warnings
cargo +1.90.0 build --release --manifest-path benches/scaling/Cargo.toml --bins

# Throughput: events, arithmetic rounds/event, events/submission, samples,
# execution layers, comma-separated response-channel pool sizes.
benches/scaling/target/release/contime-scaling-bench 524288 0 1 5 full 1,64
benches/scaling/target/release/contime-scaling-bench 524288 0 1000 5 full 1,64

# Latency: events, workers, events/submission, arrival pattern, query interval (us).
benches/scaling/target/release/latency 65536 4 1 burst 2000
```

The scaling layers are direct `compute`, preloaded real workers (`preloaded`),
streaming real workers (`workers`), router plus workers (`routed`), and the public
API (`full`). Fixed total work is distributed over 64 histories and one, two,
and four workers. Event counts must be positive multiples of 64. Each run checks
counts and order-sensitive canonical fingerprints, including with zero extra
arithmetic rounds. Response channels rotate by submission index, independently
of event IDs or batch alignment. Event records and submission vectors are built
before timing. Streaming timing includes routing/conversion allocation and
submission; final public verification and shutdown are outside timing.

The CPU-time probes support macOS and Linux. Threads are unpinned.

Latency patterns are `saturated` (unpaced), `burst` (1,024 events every 5 ms),
and `sparse` (one event per millisecond). Submissions are split at arrival-group
boundaries, so caller batch size never changes the offered arrival schedule.
Late producer wakeups catch up against the original schedule without adding drift. The probe records live
application and concurrent query latency; independent final-state verification
is outside timing. Startup idle helpers are used only as benchmark controls.

Run one benchmark at a time on an otherwise idle host. Compare matching workload,
compiler, dependency versions, and build profiles. These measure the full native
pipeline; use `cargo bench -p contime-batcher --bench batcher` to isolate batching.

Compare revisions with identical probe sources, compiler, dependencies, and build
profiles. The zero-round workload still uses an order-sensitive hash. Report
medians and spread rather than a single run.

The latency probe queries the final timestamp while processing catches up.
Frequent queries can repeatedly reconstruct unprocessed event tails, so this is
a query-stress workload. Its completion rate is not baseline application throughput.
