use contime::prelude::*;
struct Local(std::rc::Rc<()>);
impl contime::checkpoints::Event for Local {type Time=i64;fn time(&self)->i64{0}}
impl EventId for Local {fn event_id(&self)->u128{1}}
#[derive(ContimeModule)]
#[contime_module(time=i64,events=[Local])]
struct App;
fn main() {
    let _ = ConTime::<App>::start(ConTimeConfig {router_count:1,worker_count:1,placement:Placement::default(),pruning_interval:std::time::Duration::from_secs(1),history_retention:10,checkpoints:CheckpointConfig{interval:2}});
}
