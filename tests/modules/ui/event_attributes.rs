use contime::{ContimeEvent, ContimeSnapshot};

#[derive(ContimeSnapshot)]
#[contime(event = Added)]
struct MissingId;

#[derive(ContimeSnapshot)]
#[contime(event = Added, id = route, event = Removed)]
struct TwoEvents;

#[derive(ContimeSnapshot)]
#[contime(event = Added, id = route)]
#[contime(event = Added, id = other_route)]
struct DuplicateEvent;

#[derive(ContimeEvent)]
#[contime(event = Added, id = route)]
struct NotASnapshot;

fn main() {}
