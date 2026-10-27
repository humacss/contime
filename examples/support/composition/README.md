# Portable example composition

The `browser` and `wasi` modules select cases and connect their simulator
callbacks, monitoring, validation and presentation. Their Cargo packages stay
under `../../environments`, with explicit source paths into this folder.

Browser `session.rs` selects a case, `frame.rs` runs the nonblocking diagnostic
work, and `index.html` / `ui/` render its reports. The browser environment supplies
only the generic frame scheduler and JavaScript clock binding; the exported session wrapper stays in support. Generated
bindings are written to `browser/pkg/` and remain ignored.

WASI `main.rs` / `config.rs` select a case and execution mode; `workload.rs`
connects the shared diagnostic driver to stdout presentation. The WASI environment
provides the target configuration and Node/Wasmtime hosting. Computation remains
inside WebAssembly, while launcher terminal presentation remains outside it.

Native composition entry points live in `../entrypoints`.
