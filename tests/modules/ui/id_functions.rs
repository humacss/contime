use contime::{ContimeEvent, ContimeSnapshot};

#[derive(ContimeEvent)]
#[contime(id = wrong_argument)]
struct WrongArgument { #[contime(time)] at: i64 }
fn wrong_argument(_: u64) -> u128 { 0 }

#[derive(Clone, ContimeSnapshot)]
#[contime(id = wrong_result)]
struct WrongResult { #[contime(time)] at: i64 }
fn wrong_result(_: &WrongResult) -> u64 { 0 }

#[derive(ContimeEvent)]
#[contime(id = first, id = second)]
struct Duplicate { #[contime(time)] at: i64 }
fn first(_: &Duplicate) -> u128 { 0 }
fn second(_: &Duplicate) -> u128 { 1 }

fn main() {}
