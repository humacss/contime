# Account balances and provisional payments

Choose **Visualizer → 3 Account balances** in the graphical window opened by
`cargo run --example launcher`. Native, WASI, WASI Threads and Browser share this
case and its Bevy presentation. The browser preserves its per-frame work budget.

Both sides receive the same transaction deliveries. ConTime reconstructs the
account using action timestamps; the comparison evaluates requests in arrival
order. **Both sides deduplicate transaction IDs.** This isolates the ordering
difference rather than making the comparison deliberately count retries twice.

Each eight-second round supplies a 100-credit deposit, an 80-credit payment,
a 60-credit payment, two unchanged payment retries, and a 20-credit refund.
The earlier 80-credit request arrives after the 60-credit request. Initially the
60-credit request can succeed; reconciliation later accepts the earlier request
and rejects the later one when funds are insufficient. The opening round ends
with 40 credits with reconciliation and 60 credits in arrival order. Further
rounds continue on the same account, so their decisions depend on its accumulated
balance. The network schedule is scripted to make the teaching scenario repeatable.

The input timeline previews upcoming actions. Lines show action-to-delivery delay;
plus and minus symbols distinguish credits and requests. Delivery symbols travel
to both ledgers, which show provisional decisions and step plots of the balance.
Retries keep both ID and transaction contents unchanged. They reach ConTime; the
producer does not filter them out. The duplicate count shows deliveries ignored
by both views without changing either balance. Reset restarts the same scenario;
P pauses, and Back returns to the visualization chooser.

Start with [model.rs](model.rs) for the event, snapshot and insufficient-funds rule.
[setup.rs](setup.rs) adapts the lane to the shared environments.
The [shared driver](../../support/accounts/driver.rs) owns delivery, worker-status
collection, queries and validation against an independent chronological reference.
It preserves queued inputs across retention advancement, bounds pending traffic
and presentation history, and discards future actions on stop. Inputs whose action
time has passed are flushed in scheduled delivery order before final validation.

This example demonstrates **provisional reconciliation and idempotency**, not
settlement, consensus or prevention of spending across independent systems.
Nothing transfers externally. A real application must define authoritative
ordering, a reconciliation window, and when decisions become final. An event ID
identifies an unchanged transaction; changing a transaction is a different operation.
Retries in this scenario arrive within the retained history, not indefinitely
after that history has been pruned. Integer credits avoid currency rounding concerns.
