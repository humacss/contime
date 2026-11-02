use contime::{ContimeEvent,ContimeSnapshot,EventId,SnapshotId};
#[derive(ContimeEvent)]
struct E { #[contime(time)] at:i64 }
#[derive(Clone,ContimeSnapshot)]
struct S { #[contime(time)] at:i64 }
fn main() {
 let _=EventId::event_id(&E{at:0});
 let _=SnapshotId::snapshot_id(&S{at:0});
}
