# contime-api

`contime-api` isolates the work performed at ConTime's public input boundary.
It does not know what receives or processes its output.

The existing `contime` crate does not use this crate yet, and this crate is not
registered as a workspace member. For now, `contime-api` depends on `contime`
for the shared rejection contracts. That dependency can move to a core crate
when the shared contracts are extracted.

## Responsibilities

`send` accepts either owned values or existing `Arc` values, normalizes them
into `Vec<Arc<E>>`, and forwards one `InputBatch` through an opaque output
channel. Owned values receive one Arc allocation. Existing Arcs preserve their
allocation and move into the batch without being cloned. The batch owns one
rejection sender. An empty input sequence forwards nothing and drops the
sender immediately.

`apply` owns the rejection channel, delegates input forwarding to `send`, and
collects rejection messages until every downstream rejection sender has been
dropped. It sorts and deduplicates identical `(event_id, reason)` pairs before
returning them.

The crate does not inspect snapshot IDs, flatten inputs by route, hash routes,
partition work, manage memory, or process events. Its output contract is
defined locally; it does not know that a router exists. An orchestrator can
bridge this Arc-normalized batch into another independently defined boundary.

## Verification

Run the unit tests from the ConTime repository root:

```bash
cargo test --manifest-path crates/api/Cargo.toml --lib
```

Run the two inline Criterion benchmarks separately:

```bash
cargo test --manifest-path crates/api/Cargo.toml --release --lib benchmark_send -- --ignored --nocapture
cargo test --manifest-path crates/api/Cargo.toml --release --lib benchmark_apply -- --ignored --nocapture
```

## Benchmarks

The following medians were recorded locally in one release-mode run on
2026-08-28. The fixtures have compile-time assertions fixing their complete
sizes at exactly 32, 64, 208, and 1,008 bytes.

| Event bytes | Ownership | Inputs | Total | ns/input | Inputs/s |
| ---: | :--- | ---: | ---: | ---: | ---: |
| 32 | owned | 1 | 144.84 ns | 144.84 | 6,904,170 |
| 32 | shared | 1 | 111.80 ns | 111.80 | 8,944,544 |
| 64 | owned | 1 | 147.41 ns | 147.41 | 6,783,800 |
| 64 | shared | 1 | 113.78 ns | 113.78 | 8,788,891 |
| 208 | owned | 1 | 154.28 ns | 154.28 | 6,481,722 |
| 208 | shared | 1 | 110.54 ns | 110.54 | 9,046,499 |
| 1,008 | owned | 1 | 201.80 ns | 201.80 | 4,955,401 |
| 1,008 | shared | 1 | 113.58 ns | 113.58 | 8,804,367 |
| 32 | owned | 1,000 | 13.812 µs | 13.812 | 72,400,811 |
| 32 | shared | 1,000 | 487.34 ns | 0.48734 | 2,051,955,514 |
| 64 | owned | 1,000 | 14.252 µs | 14.252 | 70,165,591 |
| 64 | shared | 1,000 | 496.09 ns | 0.49609 | 2,015,763,269 |
| 208 | owned | 1,000 | 18.671 µs | 18.671 | 53,558,995 |
| 208 | shared | 1,000 | 503.51 ns | 0.50351 | 1,986,057,874 |
| 1,008 | owned | 1,000 | 59.246 µs | 59.246 | 16,878,777 |
| 1,008 | shared | 1,000 | 527.88 ns | 0.52788 | 1,894,369,933 |

Each `send` case prepares the input vector and both Crossbeam channels outside
the timed routine. The measurement includes normalization into
`Vec<Arc<E>>` and one real output-channel send. It excludes downstream receipt,
event processing, input construction, and destruction of the queued output.

The owned cases include one `Arc::new` allocation per input, so their cost
grows with event size. The shared cases begin with `Vec<Arc<E>>`; converting
each handle is the identity operation, and ownership of the existing vector
elements moves into the batch. These cases therefore measure the intended
fast path for inputs that a consumer already shares.

`api/apply/no_rejections` measured **106.00 ns**. It includes creation and
closure of the internally owned rejection channel plus empty rejection
collection. Its `send` dependency is stubbed, so it does not include input
normalization, output-channel transport, or downstream work.
