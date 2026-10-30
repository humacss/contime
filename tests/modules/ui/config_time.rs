use contime::{ConTime, ConTimeConfig, ContimeEvent, ContimeModule, ContimeSnapshot};
#[derive(ContimeEvent)]
struct Changed { time: i64, id: u64 }
#[derive(ContimeSnapshot)]
#[contime_snapshot(id = [self.id], events = [Changed], apply = {})]
struct State { time: i64, id: u64 }
#[derive(ContimeModule)]
#[contime_module(time = i64, events = [Changed], snapshots = [State])]
struct App;
fn start(config: ConTimeConfig<u64>) { let _ = ConTime::<App>::start(config); }
fn main() {}
