use contime::{ContimeEvent, ContimeSnapshot};
#[derive(ContimeEvent)]
#[contime(hash = true)]
struct OldEvent { id:u128, #[contime(time)] time:i64 }
#[derive(Clone,ContimeSnapshot)]
#[contime(apply = {})]
struct OldSnapshot { #[contime(time)] time:i64 }
fn main() {}
