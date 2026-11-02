use contime::prelude::*;
#[derive(ContimeEvent)]
struct Added { #[contime(id)] id:u128, owner:u128, #[contime(time)] at:i64 }
#[derive(ContimeEvent)]
struct Other { #[contime(id)] id:u128, owner:u128, #[contime(time)] at:i64 }
#[derive(Clone,Default,ContimeSnapshot)]
#[contime(events=[Added])]
struct State { #[contime(id)] owner:u128, #[contime(time)] at:i64 }
impl SnapshotApply for State {}
#[derive(ContimeModule)]
#[contime(snapshot=State, events=[Other])]
struct Unsupported;
#[derive(ContimeModule)]
#[contime(snapshots=[State],events=[Added])]
struct PluralOverride;
#[derive(ContimeModule)]
#[contime(snapshot=State,events=[Added,Added])]
struct Repeated;
#[derive(ContimeModule)]
#[contime(snapshots=[])]
struct Empty;
fn main() {}
