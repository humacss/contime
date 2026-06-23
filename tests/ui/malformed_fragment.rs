struct BrokenAt;
struct OnBroken;

contime::lanes! {
    mod broken_lanes;
    snapshots [BrokenAt];
    routes [
        OnBroken => BrokenAt,
    ];
}

fn main() {}
