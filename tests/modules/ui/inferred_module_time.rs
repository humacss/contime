use contime::prelude::*;
#[derive(ContimeEvent)]
struct Added { #[contime(id)] id:u128, owner:u128, #[contime(time)] at:i64 }
#[derive(Clone,Default,ContimeSnapshot)]
#[contime(events=[Added])]
struct Signed { #[contime(id)] owner:u128, #[contime(time)] at:i64 }
impl SnapshotApply for Signed {}
#[derive(ContimeEvent)]
struct AddedUnsigned { #[contime(id)] id:u128, owner:u128, #[contime(time)] at:u64 }
#[derive(Clone,Default,ContimeSnapshot)]
#[contime(events=[AddedUnsigned])]
struct Unsigned { #[contime(id)] owner:u128, #[contime(time)] at:u64 }
impl SnapshotApply for Unsigned {}
#[derive(ContimeModule)]
#[contime(snapshots=[Signed,Unsigned])]
struct Mixed;
fn main() {}
