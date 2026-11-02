use contime::{ContimeEvent, ContimeSnapshot};

#[derive(ContimeEvent)]
#[contime(id = |_| 1)]
struct Changed { #[contime(time)] at: i64 }

#[derive(Clone, ContimeSnapshot)]
#[contime(id = |_| 7)]
#[contime(event = Changed, id = wrong_route)]
struct BadRoute { #[contime(time)] at: i64 }
fn wrong_route(_: &Changed) -> u128 { 7 }

#[derive(Clone, ContimeSnapshot)]
#[contime(id = |_| 7)]
#[contime(event = Changed, id = |_, emit| emit(7))]
struct BadTime { #[contime(time)] at: i32 }

fn main() {}
