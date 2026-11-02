use contime::prelude::*;
#[derive(ContimeEvent)]
struct Added {
    #[contime(id)] id: u128,
    #[contime(time)] at: i64,
    destination: String,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.owner)]
#[contime(event = Added, id = |v| v.destination, map = |_| 7)]
struct WrongFieldType {
    owner: u128,
    #[contime(time)] at: i64,
}
#[derive(Clone, Default, ContimeSnapshot)]
#[contime(id = |v| v.owner)]
#[contime(events = [Added])]
struct MissingEventField {
    owner: u128,
    #[contime(time)] at: i64,
}
fn main() {}

#[derive(ContimeSnapshot)]
struct WrongSnapshotIdType { #[contime(id)] id: u64 }
