# Composable modules

Run `cargo run --example modules --features macros` from the repository root.
The `macros` feature supplies the three derives; `runtime` and built-in integer
`time` support are enabled by default. Manual processing additionally needs
`manual` and uses the same `App` with `ManualConTime::<App>::new(config)`.

`OrderedValue` gets a stable event ID from its contents. Repeating the same input is a
no-op. `OrderedList` and `ValueStatistics` get separate snapshot ID namespaces even
when both use list ID 7. Each snapshot declares its input events and update
rule. The derives supply `Hash` for events and `Clone` and `Default` for snapshots.

`Statistics` depends on `Values`; `App` also includes `Values` directly.
Composition visits that shared dependency once. `AppEventLanes` and
`AppSnapshotLanes` provide the resulting typed input and query alternatives.

Integer time is measured in units chosen by the application. This example uses
logical milliseconds, including negative values before its chosen origin. The
retention duration uses the same units. A request-scoped insertion response
precedes the query; global idle waiting is reserved for controlled tests.

The snapshot appends `(timestamp, value)` pairs without sorting. The first query
shows `[(-10, 'a'), (30, 'c')]`. A late input at time 10 changes the next query to
`[(-10, 'a'), (10, 'b'), (30, 'c')]`, demonstrating order-dependent reconstruction.
The statistics snapshot changes from two records to three.
