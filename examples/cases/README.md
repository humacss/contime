# Example cases

Start here to understand the application being modelled. Each case owns its
events, snapshots, lane setup and correctness rules. Simulator callbacks also
stay with their case; shared support connects them to the chosen environment.

- [Counter](counter/README.md): the README walkthrough, counter model and traffic callbacks.
  [Manual walkthrough](counter/manual.md) demonstrates caller-driven processing.
- [Chain hash](chain_hash/README.md): order-sensitive application and reference validation.
- [Client movement](client_movement/README.md): timestamped direction inputs, character snapshots and corrected paths.

Live native entry points live in [support/entrypoints/](../support/entrypoints/README.md).
The [launcher](../support/launcher/README.md) selects compatible cases, environments,
execution modes and startup settings. Shared loops and presentation live in
[support](../support/README.md).
