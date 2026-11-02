use contime::ContimeModule;
#[derive(ContimeModule)]
#[contime(modules=[Second])]
struct First;
#[derive(ContimeModule)]
#[contime(modules=[First])]
struct Second;
fn main() {}
