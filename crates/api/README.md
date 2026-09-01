# contime-api

`contime-api` isolates the work performed at ConTime's public input boundary.
It does not know what receives or processes its output.

The existing root `contime` crate does not use this crate yet, and this crate
is not registered as a workspace member. The API crate is fully isolated: it
depends only on Crossbeam and defines its own generic rejection contract.

## Responsibilities

`send` converts each input into the caller-selected input type, collects one
ordinary `Vec<I>`, and asks the caller-selected `ApplyOutput` implementation to
construct the message forwarded through an opaque output channel. `InputBatch`
remains an optional default implementation. The API does not prescribe whether
`I` is an owned value, shared
pointer, tracked pointer, or another ownership type. The batch owns one
rejection sender. An empty input sequence forwards nothing and drops the
sender immediately.

`apply` owns the rejection channel, delegates input forwarding to `send`, and
collects rejection messages until every downstream rejection sender has been
dropped. It sorts and deduplicates identical `(event_id, reason)` pairs before
returning them. Rejection reasons are generic; a downstream core or consumer
defines their meaning while this crate only requires them to be ordered.

The crate does not inspect snapshot IDs, flatten inputs by route, hash routes,
partition work, manage memory, or process events. Its output contract is
defined locally; it does not know that a router exists. An orchestrator can
implement `ApplyOutput` on a type that also implements another crate's input
trait, making the handoff compile-time checked without an API dependency.

Snapshot and event-history queries use the same ownership-neutral boundary.
Asynchronous calls forward a caller-owned response sender. Synchronous wrappers
collect result batches until every downstream sender clone is dropped.
Snapshot queries return only found boxed snapshots; event queries return owned
handles selected by the consumer's output type.

Horizon advancement follows the same closure contract. `send_advance_to`
forwards one timestamp and caller-owned completion sender. `advance_to` owns
that channel and waits until every downstream sender clone has been dropped;
there is no acknowledgement payload.

Snapshot replay listeners use only an asynchronous boundary.
`send_listen_snapshots` collects one watched timestamp and the requested
snapshot-ID set, then forwards them with a consumer-owned notification sender.
The API does not create the channel,
interpret listener notifications, or wait for distributed registration. Empty
registrations forward nothing and drop the supplied sender.

## Verification

Run the unit tests from the ConTime repository root:

```bash
cargo test --manifest-path crates/api/Cargo.toml --lib
```

Run the inline Criterion benchmarks separately:

```bash
cargo test --manifest-path crates/api/Cargo.toml --release --lib benchmark_send -- --ignored --nocapture
cargo test --manifest-path crates/api/Cargo.toml --release --lib benchmark_apply -- --ignored --nocapture
cargo test --manifest-path crates/api/Cargo.toml --release --lib benchmark_send_listen_snapshots -- --ignored --nocapture
```

## Benchmarks

Listener forwarding of one timestamp and 1,000 already-collected snapshot IDs
measured **205.28 ns**, or approximately **0.205 ns per ID**, on 2026-09-02.
The snapshot-ID vector and both channels are prepared outside the timed routine. Like the apply
send benchmark, consuming and recollecting the owned vector can reuse its
allocation; this measurement describes the API boundary only and excludes
router partitioning, worker registration, and notification delivery.

The following medians were recorded locally in one release-mode run on
2026-08-31. The fixtures have compile-time assertions fixing their complete
sizes at exactly 32, 64, 208, and 1,008 bytes.

| Event bytes | Ownership | Inputs | Total | ns/input | Inputs/s |
| ---: | :--- | ---: | ---: | ---: | ---: |
| 32 | owned | 1 | 103.87 ns | 103.87 | 9,627,419 |
| 32 | shared | 1 | 110.42 ns | 110.42 | 9,056,330 |
| 64 | owned | 1 | 104.08 ns | 104.08 | 9,607,994 |
| 64 | shared | 1 | 110.42 ns | 110.42 | 9,056,330 |
| 208 | owned | 1 | 104.81 ns | 104.81 | 9,541,074 |
| 208 | shared | 1 | 110.78 ns | 110.78 | 9,027,983 |
| 1,008 | owned | 1 | 103.68 ns | 103.68 | 9,644,290 |
| 1,008 | shared | 1 | 110.87 ns | 110.87 | 9,019,572 |
| 32 | owned | 1,000 | 87.875 ns | 0.08788 | 11.38 billion |
| 32 | shared | 1,000 | 483.52 ns | 0.48352 | 2.068 billion |
| 64 | owned | 1,000 | 85.384 ns | 0.08538 | 11.71 billion |
| 64 | shared | 1,000 | 495.15 ns | 0.49515 | 2.019 billion |
| 208 | owned | 1,000 | 105.48 ns | 0.10548 | 9.481 billion |
| 208 | shared | 1,000 | 506.20 ns | 0.50620 | 1.975 billion |
| 1,008 | owned | 1,000 | 130.84 ns | 0.13084 | 7.642 billion |
| 1,008 | shared | 1,000 | 522.25 ns | 0.52225 | 1.915 billion |

Each `send` case prepares the input vector and both Crossbeam channels outside
the timed routine. The measurement includes generic identity conversion,
collection into the forwarded vector, and one real output-channel send. It
excludes downstream receipt, input construction, and destruction of the queued
output. Rust can reuse an owned `Vec` allocation while collecting its consumed
iterator, which explains the exceptionally small amortized batch figures.

Both ownership strategies are effectively free at this boundary compared with
downstream routing and processing. The API result does not justify either
ownership strategy; it demonstrates that the generic API does not introduce a
payload-sized copy or allocation. Router fan-out is the operation that makes
shared ownership materially valuable.

`api/apply/no_rejections` measured **107.49 ns**. It includes creation and
closure of the internally owned rejection channel plus empty rejection
collection. Its `send` dependency is stubbed, so it does not include input
normalization, output-channel transport, or downstream work.

Query boundary results recorded on 2026-09-01 include one real request/response
round trip to a minimal downstream thread and synchronous result collection:

| Results | Snapshot query | Event query |
| ---: | ---: | ---: |
| 1 | 1.534 us | 1.023 us |
| 10 | 2.325 us | 1.014 us |
| 100 | 11.01 us | 1.409 us |
| 1,000 | 34.32 us | 3.724 us |

The snapshot fixture allocates one boxed value per result downstream. The event
fixture returns one contiguous vector, so the difference primarily describes
result ownership and allocation rather than request construction.
