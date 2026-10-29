# Shared example support

Application models, lane setup and simulator callbacks live under
[cases](../cases/README.md). Environment adapters live under
[environments](../environments/README.md). Support composes those pieces through
public ConTime, Monitor and Simulator APIs; it is not another library backend.

| Module | Responsibility |
| --- | --- |
| `settings.rs`, `config.rs`, `targets.rs`, `rate.rs` | Shared startup configuration, validation and exact decimal fanout settings |
| `definition.rs` | Case hooks and application time, independent of execution and presentation |
| `execution/mod.rs` | Manual/Runtime selection |
| `monitored/control.rs` | Bounded Maximum production and worker-monitor completion feedback |
| `monitored/driver.rs`, `monitored/backend.rs` | Shared diagnostic loop and monitoring bridges to generic environment execution |
| `movement/` | Real-time delayed delivery, monitored historical queries, fading states and the client map |
| `entrypoints/`, `composition/` | Case selection and host composition for native, browser and WASI |
| `monitored/reference.rs`, `monitored/sampling.rs` | Independent canonical state validation and simulator destination selection |
| `monitored/metrics.rs`, `monitored/wire.rs` | Presentation-neutral metrics, snapshot comparisons and bounded WASI transport |
| `monitored/ui.rs`, `ui/` | Settings, worker table, state inspection, input and terminal restoration |
| `launcher/` | Central entry point, environment selection, child hosting and browser serving |
| `portable/mod.rs` | Shared settings and formatting without terminal or host-loop dependencies |
| `execution/portable.rs`, `load/verification.rs` | Finite late-event, duplicate, rejection and pruning regression scenarios |
| `format.rs`, `exit.rs` | Shared numeric formatting and standalone WASI exit helper |

## One production policy

All live cases use simulator iterators and worker-monitor feedback. Runtime and
Manual hosts use the same bounded windows. The browser reuses the controller,
Manual backend and reference checks through its nonblocking frame adapter.
Counter and hash processing stays in WebAssembly in WASI; the launcher owns its
terminal UI. Browser clock bindings stay in the environment; case selection and presentation
stay in support composition. See
[environments/browser](../environments/browser/README.md).

Maximum starts from a 100,000 source-events/s probe and confirms a sealed window
only after every worker reports the exact accepted application boundary, the
required processing target and no pending replay histories. Replay totals alone
never imply completion. Prompt windows increase the next probe; slower windows
reduce it. Numeric rates pace source events and optionally cap outstanding source
events. An event with several destinations still uses one source slot.

Queries inspect state; they do not release production credit. Counters validate
once after monitor-confirmed drain. Hashes validate complete digests during the
run and at shutdown. Pending validation freezes new inputs and time
advancement, preserving its canonical prefix and retention boundary.

## Execution and presentation

Manual performs one existing `process()` per host-loop turn. It queues explicit
pruning after advancement about every 100 ms and reserves queue slots for control
and query operations. The generic environment execution adapter delegates Runtime to existing native APIs. Neither
monitor collection nor rendering drives worker execution.

Browser frames check their time and step limits between complete operations;
they never wait or sleep. The allowance is soft because an individual operation
may overrun it. Native/WASI loops may yield briefly when no work can be submitted.

The terminal reader runs on its own thread with bounded input and joined
shutdown. Worker totals and cumulative average rates are rendered from monitor
readings. Native snapshot tables format visible rows only. WASI reports include
at most 64 snapshot comparisons, retaining complete digests; the launcher labels
this as a preview and still validates every snapshot in the guest. The stdout
protocol has a 1 MiB record bound and a two-record reader queue. Browser hash
presentation shows the first 20 comparisons; all configured snapshots are validated.

## Validation and shutdown

Canonical references account for actual target applications. Validation rejects
missing, duplicate or unknown states and mismatched counts, values or full
hashes. Hash groups are submitted in reverse order, including at low rates when
a pair fits; one-slot windows and final single-event groups cannot be reversed.

Native hosts stop production, confirm outstanding work, validate final state,
restore the terminal and shut down their backend normally before the summary.
The standalone WASI guest flushes its validated final report before process exit.
Browser sessions retain normal ownership and pause/resume behavior.

The finite portable scenario independently checks late replay, duplicate
rejection, pruning and frame allowances.

Run focused checks from the repository root:

```sh
cargo test --examples
cargo clippy --examples --bench monitor_ui -- -D warnings
cargo bench --features monitor --bench monitor_ui -- --test
```
