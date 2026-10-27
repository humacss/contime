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

Older measurements predate the replay scheduling fix and these probe corrections.
They are not directly comparable: even the zero-round workload now uses an
order-sensitive hash. Compare revisions with identical corrected probe sources
and lockfiles; report medians and spread rather than a single run.
Historical measurements and discarded experiment patches are local working
notes under the ignored `docs/local-investigations/native-scaling/` directory.

A matched 2026-10-28 comparison of pre-batching `49b7f61` and candidate `7d436d6`
used these corrected probes, Rust 1.90.0, identical external dependency versions,
and alternating samples on an unpinned M3 Pro under background load. The baseline
also received the published Traitstub compatibility patch (`5d11822`). For 524,288
events over 64 snapshot IDs, five-sample medians with one shared response channel:

| Workers | Individual submissions | Batches of 1,000 |
| --- | --- | --- |
| 1 | 3.95 M events/s | 4.03 M events/s |
| 2 | 6.93 M events/s | 7.97 M events/s |
| 4 | 8.49 M events/s | 17.88 M events/s |

The candidate improves four-worker individual-submission completion time by about
2×; batched medians are within about 2% of the baseline. Both revisions already
contain the replay scheduling fix. These finite-work results describe this machine
and workload, not a sustained performance guarantee.

The latency probe queries the final timestamp while processing is still catching
up. Frequent queries can repeatedly reconstruct unprocessed event tails; at a 2 ms
query interval this is a query-stress benchmark. In a one-worker control, the
candidate completed 262,144 individual submissions in a median 74 ms without
periodic queries, 75 ms with a 50 ms interval, and 3.8 s with a 2 ms interval.
Those short controls recorded zero, one, and roughly 1,200 queries respectively.
Do not interpret the query-stress completion rate as baseline application throughput.
