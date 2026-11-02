use contime::{ContimeEvent, ContimeSnapshot};

#[derive(ContimeEvent)]
struct Multiple {
    #[contime(id)] first: u128,
    #[contime(id)] second: u128,
}
#[derive(ContimeEvent)]
#[contime(id = |v| v.id)]
struct Ambiguous { #[contime(id)] id: u128 }
#[derive(ContimeEvent)]
#[contime(map = |id| id)]
struct FieldMap { #[contime(id)] id: u128 }
#[derive(ContimeSnapshot)]
#[contime(id = |v| v.id + 1, map = |id| id)]
struct Calculated { id: u128 }
#[derive(ContimeSnapshot)]
#[contime(id = |v| v.id)]
#[contime(events = [Added], map = |id| id)]
struct PluralOverride { id: u128 }
#[derive(ContimeSnapshot)]
#[contime(id = |v| (v.first, v.second))]
#[contime(event = Added, id = |v| v.id)]
struct Shape { first: u128, second: u128 }
#[derive(ContimeSnapshot)]
#[contime(id = |v| v.id)]
#[contime(events = [Added])]
#[contime(event = Added)]
struct Duplicate { id: u128 }
fn main() {}

#[derive(ContimeSnapshot)]
struct MultipleSnapshotIds {
    #[contime(id)] first: u128,
    #[contime(id)] second: u128,
}
#[derive(ContimeSnapshot)]
#[contime(id = |v| v.id)]
struct AmbiguousSnapshotId { #[contime(id)] id: u128 }
#[derive(ContimeSnapshot)]
#[contime(map = |id| id)]
struct SnapshotFieldMap { #[contime(id)] id: u128 }
