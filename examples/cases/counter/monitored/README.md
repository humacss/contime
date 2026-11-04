# Counter simulator callbacks

`generation.rs` supplies the two callbacks used by `contime-simulator::EventStream`:
create a counter event with a unique ID and timestamp, then assign its distinct
snapshot destinations using the configured fanout and traffic skew.

The counter workload is retained for internal testing and benchmarking. It uses
the shared monitored host and can exercise Runtime and Manual processing without
being offered in the public launcher. See the [counter guide](../README.md) for
the model and [shared support](../../../support/monitored/README.md) for its controller.

## Verification

```sh
cargo test --all-features --test counter_internal
cargo clippy --all-features --test counter_internal -- -D warnings
```

Tests preserve fixed/Maximum production, multiple workers and destinations,
Manual execution, short retention, unique event IDs and concentrated traffic.
Controller regressions remain in shared support.
