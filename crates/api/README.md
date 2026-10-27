# contime-api

`contime-api` isolates the work performed at ConTime's public input boundary.
Its default channel helpers do not know what receives or processes their output.

Runtime uses the default helpers to submit its protocol messages. The optional
`manual` feature adds callback-based request adapters for the worker contracts;
it owns no worker, storage, queue or processing loop. The root manual facade
composes those adapters with Core storage and the thin manual driver.

Default channel helpers use Crossbeam. Enabling `manual` also depends on
`contime-worker`; callbacks may capture thread-local state and never wait on a
channel. They run only after a worker begins processing their request. Empty
queries complete with an empty vector; dropping unprocessed requests does not
invoke callbacks. Callbacks must not block waiting for the same worker.

The synchronous helpers here are independent functions. The root engine's
`apply` and `advance_to` methods enqueue work asynchronously. In the composed
pipeline, closing an apply rejection stream acknowledges insertion; it does
not establish replay completion.

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

Inline benchmarks measure request construction, identity conversion, collection,
and channel sends. Query benchmarks include owned result collection. Fixtures
are prepared outside timing; downstream routing and replay are separate costs.
Run the inline commands above, or the public query benchmark:

```sh
cargo bench -p contime-api --bench query
```
