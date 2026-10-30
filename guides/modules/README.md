# Application modules

[Overview](../../README.md) · [Explicit traits](../usage/README.md) · [Time](../../crates/time/README.md)

The `macros` feature re-exports `ContimeModule` from `contime-macros` in
ConTime 0.1.2. Events and snapshots use ordinary trait implementations. Enable it with
`contime = { version = "0.1.2", features = ["macros"] }`.

Start with the [step-by-step ordered-list walkthrough](../../examples/ordered_list.rs)
for individual late insertions and intermediate assertions. Then run the complete
[dependent-module example](../../examples/modules.rs):

```sh
cargo run --example modules --features macros
```

## Define data and application rules

IDs, destinations, initialization and application rules are ordinary Rust.
Import common types with `use contime::prelude::*;`. Implement timestamp access
through `checkpoints::Event` and `checkpoints::Snapshot`, and use ordinary Rust
`Clone` and optional `Default` for snapshots. ConTime chooses no identity
algorithm or namespace.

```rust,ignore
use contime::prelude::*;

struct Added {
    time: i64,
    id: u128,
    counter_id: u128,
    amount: i64,
}
impl contime::checkpoints::Event for Added {
    type Time = i64;
    fn time(&self) -> Self::Time {
        self.time
    }
}

impl EventId for Added {
    fn event_id(&self) -> u128 {
        self.id
    }
}

#[derive(Clone, Default)]
struct Counter {
    time: i64,
    counter_id: u128,
    value: i64,
}
impl contime::checkpoints::Snapshot for Counter {
    type Time = i64;
    fn time(&self) -> &Self::Time {
        &self.time
    }
    fn set_time(&mut self, time: Self::Time) {
        self.time = time;
    }
}

impl SnapshotId for Counter {
    fn snapshot_id(&self) -> u128 {
        self.counter_id
    }
}
impl SnapshotApply for Counter {
    fn define(events: &mut EventSetBuilder<Self>) {
        events.event::<Added>(|event, emit| emit(event.counter_id));
    }
    fn create(snapshot_id: u128, _event: EventRef<'_>) -> Self {
        Self { counter_id: snapshot_id, ..Self::default() }
    }
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        for event in batch.events::<Added>() {
            self.value += event.amount;
        }
    }
}
```

`EventId::event_id(&self)` returns an event ID unchanged. Use a field, a custom
function, or an identity library. Payloads need no `Hash` or serialization unless
your algorithm requires them. Use the standard `#[derive(Hash)]` when needed. You own uniqueness and stability across types.
The [ordered-list example](../../examples/ordered_list.rs) chooses Content ID 1.0;
the live counter uses supplied numeric IDs.

### Routing and snapshot identity

`SnapshotApply::define` registers accepted event types and destination-ID
callbacks. The callback receives a borrowed event and an ID emitter, and can
emit zero, one or many IDs. For example:

```rust,ignore
events.event::<SharedChange>(|event, emit| {
    for &id in &event.counter_ids {
        emit(id);
    }
});
```

One event may be registered by several snapshot types, with different routing
callbacks. The module's event lanes must contain every event registered by its
snapshots. Definitions are cached and validated before Runtime starts threads
or Manual starts processing. Duplicate registrations keep the first callback.
Callbacks are shared for the lifetime of the definition. Any captured values
must be `Send + Sync + 'static`; this does not add those bounds to events. Keep
routing deterministic, using event fields or immutable configuration.

`create(snapshot_id, event)` initializes the selected history once from its
first input, before applying that input. Its `snapshot_id(&self)` must match the
selected destination. Store the ID directly, or initialize identifying fields
from the event via `event.get::<EventType>()`. Composite identities and hashing
are application choices. Routing never initializes candidate snapshots.

### Typed timestamp batches

`apply_batch` receives one complete effective timestamp bucket. Use
`events::<Added>()`, `events::<Cleared>()`, and other typed views in a deliberate
phase order; do not depend on cross-type event interleaving. For example, handle
all clears first, then additions. Views preserve canonical ordering within each
type and can be read repeatedly without consuming each other. ConTime keeps
distinct timestamps separate during live replay, historical queries and pruning.
Late insertion reconstructs state using the same phase rules.

The context also exposes `snapshot_id()`, `time()`, and `history_event_count()`.
The count is the number of raw canonical events preceding this timestamp batch,
not the count of any particular type. Views borrow payloads without cloning or
boxing them. Up to eight event references stay inline; larger buckets allocate
reference storage. Registration allocates metadata once per definition.

Runtime type identification requires `'static` event types, but event values
are only borrowed for the batch. This adds no `Clone`, `Send` or `Sync` bound;
thread-local values remain supported in Manual. For payload types borrowing
non-static data, the lower-level `checkpoints::ApplyEvents` contract remains
available with its original iterator interface.

The old snapshot `key`, `events`, `create` and `apply` attributes are rejected.
Put these rules in the builder and trait methods instead. Multi-target application
shares payloads across histories; it is not a cross-worker transaction.

## Compose once and start one instance

```rust,ignore
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added], snapshots = [Counter])]
struct Counters;

#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [Counters], snapshots = [CounterStatistics])]
struct Statistics;

#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [Counters, Statistics])]
struct App;

let contime = contime::ConTime::<App>::start(config)?;
```

Each module explicitly declares its time type. Its events, snapshots and nested
modules must use that same Rust type. Composition checks this during compilation,
flattens transitive definitions, and deduplicates shared registrations. Cyclic module
dependencies are compilation errors.

The generated `AppEventLanes` accepts `.into()` conversions from registered events.
Queries return `AppSnapshotLanes`, whose variants contain the registered states.
These types belong to the module; they are not separate construction parameters.

With `manual` enabled, use `manual::ManualConTime::<App>::new(config)` and call
`process()` from the host loop. Both execution modes use the same module and
application contracts. Low-level adapters can use `RawConTime<I, S, W>` and
`manual::RawManualConTime<I, S, W>` directly when adapting existing lane-level implementations.

## Registration names and aliases

A module lists its own event and snapshot types and its child modules. Child
registrations are included automatically; ConTime accepts one final module.
The macro generates flat enums and deduplicates repeated registration names,
including shared dependencies reached through several modules. It emits Rust
type-equality checks before merging registrations with the same name.

A bare type uses the last part of its path as its registration and variant name.
Keep that name consistent across the composed application. For an import alias,
retain the shared registration name explicitly:

```rust,ignore
use Added as Incoming;
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Added = Incoming], modules = [Counters])]
struct App;
```

The same `Name = Type` syntax works in `snapshots`. Distinct types with the same
short name, or distinct generic specializations, need different names, for example
`events = [Small = Number<u32>, Large = Number<u64>]`.

**Known limitation:** the macro cannot discover that different aliases name the
same Rust type. Registering it under different names prevents deduplication and
causes conflicting generated `From` implementations at compile time. Use one
registration name per concrete type; no duplicate lanes run silently.

## Time and observations

The default `time` feature implements common integer timestamps. `i64` uses its
full signed range, with no special boundary at zero. Retention must be nonnegative
and uses the application's chosen units. Without built-ins, implement
`time::Timestamp` and `time::AdvanceTime` for your own type and explicitly provide
its minimum. Runtime and Manual both start at that minimum.

Submission queues work. Queries observe retained admitted history and do not wait
for global idle. The example uses a request-scoped admission response; global idle
waits remain test/benchmark helpers. Starting a composed module starts one instance,
not one worker topology for each dependency.

## Updating from 0.1.1

The module API changes the public construction types. Use
`ConTime::<App>::start(config)` or `manual::ManualConTime::<App>::new(config)`
when supplying a module. Existing implementations that directly supply event and
snapshot lanes can keep those contracts and rename their instances to
`RawConTime<I, S, W>` or `manual::RawManualConTime<I, S, W>`; their wrapper arguments
remain explicit.

Custom time types now implement `Timestamp::minimum()` and
`AdvanceTime::is_valid_retention()` in addition to predecessor and retention
subtraction. The minimum is the true domain lower bound, not `Default` or zero.
Retention intervals must be nonnegative. For built-in integer time, leave the
`time` feature enabled; a Manual-only integer configuration uses
`default-features = false, features = ["manual", "time"]`. Omit `time` only when
providing the time traits for an application-owned type.
