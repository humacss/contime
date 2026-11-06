//! Consider several equally trusted sources reporting the same event with
//! different timestamps. This simple policy chooses the timestamp reported by
//! the most sources, illustrating how context can guide duplicate selection.
//! The event ID identifies the shared event; it does not include the source.
//! Run with `cargo run --example duplicate_policy --features macros`.
use contime::{ApplyContext, ContimeEvent, ContimeModule, ContimeSnapshot, DuplicateDecision, EventDuplicatePolicy, SnapshotApply};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

#[derive(Clone, ContimeEvent)]
#[contime(duplicate_policy = manual)]
struct Message {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
    text: &'static str,
    source: u8,
}

// Context belongs to one snapshot; each event ID has its own vote counts.
#[derive(Default)]
struct MessageContext {
    events: HashMap<u128, Votes>,
}
struct Votes {
    sources: HashMap<i64, HashSet<u8>>,
    winner: (i64, usize),
}
impl EventDuplicatePolicy for Message {
    type Context = MessageContext;
    fn create_context() -> Self::Context {
        MessageContext::default()
    }
    fn on_admit(event: &Self, context: &mut Self::Context) {
        context
            .events
            .insert(event.id, Votes { sources: HashMap::from([(event.time, HashSet::from([event.source]))]), winner: (event.time, 1) });
    }
    fn prune_context(event: &Self, context: &mut Self::Context) {
        context.events.remove(&event.id);
    }
    fn select_canonical(incoming: &Self, context: &mut Self::Context) -> DuplicateDecision<Self> {
        let votes = context.events.get_mut(&incoming.id).expect("initial admission recorded");
        let sources = votes.sources.entry(incoming.time).or_default();
        if !sources.insert(incoming.source) {
            return DuplicateDecision::KeepExisting;
        }
        let count = sources.len();
        let (winner_time, winner_count) = votes.winner;
        if count > winner_count || (count == winner_count && incoming.time < winner_time) {
            votes.winner = (incoming.time, count);
            if incoming.time != winner_time {
                return DuplicateDecision::ReplaceExisting { existing_time: winner_time, event: incoming.clone() };
            }
        }
        DuplicateDecision::KeepExisting
    }
}

#[derive(Clone, Default, ContimeSnapshot)]
#[contime(events = [Message])]
struct Inbox {
    #[contime(id)]
    id: u128,
    #[contime(time)]
    time: i64,
    messages: Vec<(i64, &'static str)>,
}
impl SnapshotApply for Inbox {
    fn apply_batch(&mut self, batch: impl ApplyContext<Self>) {
        self.messages.extend(batch.events::<Message>().map(|event| (event.time, event.text)));
    }
}
#[derive(ContimeModule)]
#[contime(snapshots = [Inbox])]
struct App;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = contime::ConTimeConfig {
        router_count: 1,
        worker_count: 1,
        placement: contime::Placement::default(),
        pruning_interval: Duration::from_secs(60),
        history_retention: 1000,
        checkpoints: contime::checkpoints::CheckpointConfig { interval: 32 },
    };
    let contime = contime::ConTime::<App>::start(settings)?;
    // One source reports 10. Two sources report 20, so 20 wins.
    // A repeated report from source 2 at 20 adds no vote.
    let reports = [(1, 10), (2, 20), (2, 20), (3, 20)];
    for (source, time) in reports {
        contime.apply([Message { id: 7, time, text: "hello", source }.into()]).map_err(api_error)?;
        contime.wait_until_idle(Duration::from_secs(5))?;
    }
    contime.advance_to(40).map_err(api_error)?;
    contime.wait_until_idle(Duration::from_secs(5))?;
    for snapshot in contime.query_at(25, [7]).map_err(api_error)? {
        let AppSnapshotLanes::Inbox(inbox) = &*snapshot;
        println!("Messages at time 25: {:?}", inbox.messages);
    }
    contime.shutdown();
    Ok(())
}

fn api_error(error: contime::ApiError) -> std::io::Error {
    std::io::Error::other(format!("{error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: u128, source: u8, time: i64) -> Message {
        Message { id, time, text: "hello", source }
    }

    #[test]
    fn counts_each_source_once_per_timestamp_and_selects_the_most_supported() {
        let current = report(7, 1, 10);
        let mut context = Message::create_context();
        Message::on_admit(&current, &mut context);
        assert!(matches!(Message::select_canonical(&report(7, 2, 20), &mut context), DuplicateDecision::KeepExisting));
        assert!(matches!(Message::select_canonical(&report(7, 2, 20), &mut context), DuplicateDecision::KeepExisting));
        assert!(
            matches!(Message::select_canonical(&report(7, 3, 20), &mut context), DuplicateDecision::ReplaceExisting { existing_time: 10, event } if event.time == 20)
        );
        // Source 1 can also support 20, but cannot vote for 20 twice.
        assert!(matches!(Message::select_canonical(&report(7, 1, 20), &mut context), DuplicateDecision::KeepExisting));
        assert!(matches!(Message::select_canonical(&report(7, 1, 20), &mut context), DuplicateDecision::KeepExisting));
        assert_eq!(context.events[&7].winner, (20, 3));
        assert_eq!(context.events[&7].sources[&10].len(), 1);
    }

    #[test]
    fn ties_favor_earlier_time_and_event_ids_have_separate_counts() {
        let mut context = Message::create_context();
        let current = report(7, 1, 20);
        Message::on_admit(&current, &mut context);
        assert!(
            matches!(Message::select_canonical(&report(7, 2, 10), &mut context), DuplicateDecision::ReplaceExisting { existing_time: 20, event } if event.time == 10)
        );
        let other = report(8, 1, 30);
        Message::on_admit(&other, &mut context);
        assert!(matches!(Message::select_canonical(&report(8, 2, 40), &mut context), DuplicateDecision::KeepExisting));
        assert_eq!(context.events[&7].winner, (10, 1));
        assert_eq!(context.events[&8].winner, (30, 1));
    }
}
