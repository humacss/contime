use contime::{ContimeEvent,ContimeSnapshot};
#[derive(ContimeEvent)]
struct Missing { at:i64 }
#[derive(Clone,ContimeSnapshot)]
struct Duplicate { #[contime(time)] first:i64, #[contime(time)] second:i64 }
#[derive(ContimeEvent)]
struct Unknown { #[contime(unknown)] at:i64 }
#[derive(ContimeEvent)]
enum NotAStruct { A }
fn main() {}
