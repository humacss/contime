struct FirstAt;
struct SecondAt;

mod first {
    pub struct Shared;
}

mod second {
    pub struct Shared;
}

contime::lanes! {
    mod broken_lanes;
    snapshots [
        FirstAt,
        SecondAt,
    ];
    routes [
        first::Shared => [FirstAt],
        second::Shared => [SecondAt],
    ];
}

fn main() {}
