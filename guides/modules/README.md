# Application modules

[Overview](../../README.md) · [Explicit traits](../usage/README.md) · [Time](../../crates/time/README.md)

The `macros` feature re-exports `ContimeEvent`, `ContimeSnapshot`, and
`ContimeModule` from `contime-macros` in ConTime 0.1.2. Enable them with
`contime = { version = "0.1.2", features = ["macros"] }`.

Run the complete [dependent-module ordered-list example](../../examples/cases/modules/main.rs):

```sh
cargo run --example modules --features macros
```

## Define data and application rules

An event supplies time and stable identity. The event derive implements `Hash`
automatically for default whole-event identity. Use `#[contime_event(id = self.key)]`
to hash selected identity data instead, or `hash = false` with a handwritten `Hash`.
IDs use typed local content hashing, with no serialization requirement. They are
not stable wire identities across compiler/type changes; choose explicit protocol
identity rules for data exchanged between different programs.

A snapshot declares stable identity fields, its accepted events, and `apply = { ... }`.
The macro supplies the signature, a generated borrowed event enum, `batch`, and
`Clone`/`Default`. Use `clone = false` or `default = false` to keep handwritten
implementations. Its default constructor is initialization only; ConTime sets the
initial timestamp from the actual time domain minimum.

```rust,ignore
#[derive(ContimeSnapshot)]
#[contime_snapshot(id = [self.counter_id], events = [Added], apply = {
    for event in batch.events {
        let CounterEvent::Added(event) = event;
        self.value += event.amount;
    }
})]
struct Counter { counter_id: u64, time: i64, value: i64 }
```

The identity fields are read from each event by default. An explicit relationship
can map different fields, such as `Added => [self.owner]`, or visit several IDs
with `Added => each(self.targets, |target| [target])`. Generated
`Counter::snapshot_id(counter_id)` uses the exact same typed identity calculation.
Two snapshot types have distinct identity namespaces. Events remain shared between
histories; multi-target application is not a cross-worker transaction.

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
