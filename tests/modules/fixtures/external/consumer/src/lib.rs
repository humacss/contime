use history::{ContimeModule, Input};
#[derive(ContimeModule)]
#[contime_module(time=i64,modules=[contime_domain_one::Module,contime_domain_two::Module])]
pub struct App;
#[test]
fn both_lanes_survive() {
    let a: AppEventLanes = contime_domain_one::First { time: -1, id: 7 }.into();
    let b: AppEventLanes = contime_domain_two::Second { time: -1, id: 7 }.into();
    let mut ids = Vec::new();
    a.snapshot_ids(&mut |id| ids.push(id));
    b.snapshot_ids(&mut |id| ids.push(id));
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
}
