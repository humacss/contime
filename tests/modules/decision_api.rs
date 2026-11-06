use contime::{ContimeEvent, DuplicateDecision, EventDuplicatePolicy};
#[derive(Clone, ContimeEvent)]
#[contime(duplicate_policy = manual)]
struct Message {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
}
struct Context {
    first: Option<i64>,
}
impl EventDuplicatePolicy for Message {
    type Context = Context;
    fn create_context() -> Self::Context {
        Context { first: None }
    }
    fn on_admit(event: &Self, context: &mut Self::Context) {
        context.first = Some(event.time);
    }
    fn select_canonical(incoming: &Self, context: &mut Self::Context) -> DuplicateDecision<Self> {
        if incoming.time < context.first.unwrap() {
            DuplicateDecision::ReplaceExisting { existing_time: context.first.replace(incoming.time).unwrap(), event: incoming.clone() }
        } else {
            DuplicateDecision::KeepExisting
        }
    }
}
#[test]
fn custom_policy_uses_admitted_context_and_named_decisions() {
    let mut context = Message::create_context();
    Message::on_admit(&Message { id: 7, time: 20 }, &mut context);
    assert!(matches!(
        Message::select_canonical(&Message { id: 7, time: 10 }, &mut context),
        DuplicateDecision::ReplaceExisting { existing_time: 20, .. }
    ));
}
