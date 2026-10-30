use contime::ContimeModule;

#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [Second])]
struct First;

#[derive(ContimeModule)]
#[contime_module(time = i64, modules = [First])]
struct Second;

fn main() {}
