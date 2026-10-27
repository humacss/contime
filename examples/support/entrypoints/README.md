# Native example composition

These entry points select case definitions and connect them to shared support.
They are composition roots, not environment implementations. ConTime startup and
native process lifetime come from the generic environment adapters.

| File | Cargo example | Case |
| --- | --- | --- |
| `counter.rs` | `counter_live` | Counter |
| `manual_counter.rs` | `manual_counter_live` | Counter, defaulting to Manual |
| `chain_hash.rs` | `chain_hash` | Chain hash verification |
| `client_movement.rs` | `client_movement` | Client movement |

The launcher selects these commands. Case code lives in `../../cases`; shared
simulation and monitoring composition lives in `../monitored`. Self-contained
API walkthroughs stay beside their cases.
