# contime-macros

Optional derive macros for [contime](https://github.com/humacss/contime), a Rust
library for building continuous-time state from event streams. Most applications
use these helpers through `contime` rather than depending on this crate directly:

```toml
[dependencies]
contime = { version = "0.1.2", features = ["macros"] }
```

Import `contime::prelude::*` to use the derives and application traits. Each
helper is optional. You can implement the same traits by hand when your application
needs custom behavior.

## A small example

An event describes a change. A snapshot owns the state, registers its event types,
and applies them. A module brings the snapshots together for `contime`.

```rust,ignore
use contime::prelude::*;

#[derive(ContimeEvent)]
struct Value {
    #[contime(id)]
    event_id: u128,
    list_id: u128,
    #[contime(time)]
    time: i64,
    value: char,
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(events = [Value])]
struct List {
    #[contime(id)]
    list_id: u128,
    #[contime(time)]
    time: i64,
    values: Vec<char>,
}

impl SnapshotApply for List {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Value>() {
            self.values.push(event.value);
        }
    }
}

#[derive(ContimeModule)]
#[contime(snapshots = [List])]
struct Lists;
```

`Value` has its own event ID. Its `list_id` selects the `List` snapshot with the
same ID. The derives provide the time and identity implementations, event
registration, and snapshot creation. `Clone` and `Default` are separate derives.
The application supplies the update rule through `SnapshotApply`.

See the [complete example](https://github.com/humacss/contime/blob/main/examples/readme.rs)
for configuration, event submission, queries, and shutdown.

## Time and IDs

Use `#[contime(time)]` on the timestamp field of an event or snapshot. Use one
`#[contime(id)]` field for a directly stored `u128` ID. An event ID identifies
an event, while a snapshot ID identifies the history it belongs to.

For a composite ID, select the source fields on the struct and optionally map
them to a `u128`:

```rust,ignore
#[contime(id = |value| (value.owner, value.list), map = |fields| make_id(fields))]
```

Selectors accept direct fields or tuples of fields. Borrow non-`Copy` fields with
`&value.field`. A `map` requires a struct-level selector and cannot be combined
with a field-level ID marker. A standalone ID function taking `&Self` is also
supported when no map or automatic event registration is needed.

## Registering events

`#[contime(events = [Added, Removed])]` registers each event using the snapshot's
ID fields and optional map. The selected fields must exist on each event so the
helper can route it and initialize the snapshot.

Use a singular registration to override the selector or map for one event:

```rust,ignore
#[contime(event = Forwarded, id = |value| value.destination)]
```

The event and snapshot selectors must have matching shapes. Automatic creation
requires the snapshot to implement `Default` and its identity fields to implement
`Clone`. For custom initialization or routing, implement `SnapshotEvents` and
`SnapshotCreate` manually. `SnapshotApply` is always a separate implementation;
its default method leaves application state unchanged.

See the [event registration example](https://github.com/humacss/contime/blob/main/examples/event_registration.rs)
for an override, and the [composite IDs example](https://github.com/humacss/contime/blob/main/examples/composite_ids.rs)
for a mapped ID.

## Composing modules

Use `#[contime(snapshots = [List, Other])]` for default registrations. Nested
modules can be added with `#[contime(modules = [Shared])]`. A singular
`#[contime(snapshot = Audit, events = [Added])]` selects an event subset for
one snapshot. The module infers its time and event lanes from the registered
snapshots and checks that their time types agree.

Registration names deduplicate shared types. Use a consistent name for the same
type throughout a composition. Different aliases for one type cannot be
recognized automatically and can lead to conflicting generated implementations.
When different types have the same short name, provide distinct names explicitly,
for example `events = [First = first::Changed, Second = second::Changed]`.

See the [module guide](https://github.com/humacss/contime/blob/main/guides/modules/README.md),
[module example](https://github.com/humacss/contime/blob/main/examples/modules.rs),
and [event subset example](https://github.com/humacss/contime/blob/main/examples/module_subsets.rs).
