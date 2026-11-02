use contime::ContimeSnapshot;
#[derive(ContimeSnapshot)]
struct MissingClone { #[contime(time)] at:i64 }
fn main() {}
