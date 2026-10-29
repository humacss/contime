# Example environments

Environment adapters provide host capabilities to injected application work.
They have no case registry and do not import case definitions, generate traffic,
collect monitor readings or validate application state. Support composes those
pieces and supplies them to the environment.

| Environment | Host responsibilities | Execution |
| --- | --- | --- |
| [Native](native/README.md) | Generic process cancellation and terminal lifetime | Runtime or Manual |
| [WASI](wasi/README.md) | WASI target and host transport; composition stays in support | Manual on standard WASI; Runtime or Manual on threaded WASI |
| [Browser](browser/README.md) | WebAssembly clock bindings and generic frame scheduling | Manual with a configurable frame budget |

Use `cargo run --release --example launcher` from the repository root to select a
case, environment, execution mode and settings. Its entry point and shared
selection logic live under [support/launcher](../support/launcher/README.md).
Shared `execution.rs` starts Runtime/Manual instances from supplied contracts and
execution settings. Portable compositions live in
[`support/composition`](../support/composition/README.md), and native entry points
in [`support/entrypoints`](../support/entrypoints/README.md).

Application models remain under [cases](../cases/README.md), and reusable
configuration, control, validation and UI remain under [support](../support/README.md).
