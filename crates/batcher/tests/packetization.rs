use contime_batcher::{drain, Batch, Batcher, Request, MAX_REQUESTS};
use std::collections::VecDeque;
use std::rc::Rc;

struct Segment {
    items: Vec<usize>,
    context: Rc<()>,
}
impl Batch for Segment {
    type Item = usize;
    type Context = Rc<()>;
    fn into_parts(self) -> (Vec<usize>, Rc<()>) {
        (self.items, self.context)
    }
    fn from_parts(items: Vec<usize>, context: Rc<()>) -> Self {
        Self { items, context }
    }
    fn items(&mut self) -> &mut Vec<usize> {
        &mut self.items
    }
    fn can_merge(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.context, &other.context)
    }
}
enum Message {
    Apply(Segment),
    Boundary,
}
impl Request for Message {
    type Batch = Segment;
    fn into_batch(self) -> Result<Segment, Self> {
        match self {
            Self::Apply(b) => Ok(b),
            other => Err(other),
        }
    }
}
fn segment(items: impl IntoIterator<Item = usize>, context: &Rc<()>) -> Segment {
    Segment { items: items.into_iter().collect(), context: context.clone() }
}
#[test]
fn splits_large_requests_and_preserves_completion_ownership() {
    let context = Rc::new(());
    let mut packets = Vec::new();
    let mut batcher = Batcher::new(
        |packet| {
            packets.push(packet);
            Ok::<_, ()>(())
        },
        4,
    );
    batcher.push(segment(0..5, &context)).unwrap();
    batcher.flush().unwrap();
    drop(batcher);
    assert_eq!(packets.iter().map(|p| p.iter().map(|s| s.items.len()).sum::<usize>()).collect::<Vec<_>>(), [4, 1]);
    assert_eq!(packets.iter().flatten().flat_map(|s| s.items.iter().copied()).collect::<Vec<_>>(), (0..5).collect::<Vec<_>>());
    assert_eq!(Rc::strong_count(&context), 3);
    drop(packets.remove(0));
    assert_eq!(Rc::strong_count(&context), 2);
    drop(packets);
    assert_eq!(Rc::strong_count(&context), 1);
}
#[test]
fn combines_only_compatible_adjacent_segments_and_drops_empty_requests() {
    let a = Rc::new(());
    let b = Rc::new(());
    let mut packets = Vec::new();
    let mut batcher = Batcher::new(
        |packet| {
            packets.push(packet);
            Ok::<_, ()>(())
        },
        8,
    );
    for (ids, ctx) in [(vec![0], &a), (vec![], &b), (vec![1], &a), (vec![2], &b), (vec![3], &a)] {
        batcher.push(segment(ids, ctx)).unwrap();
    }
    batcher.flush().unwrap();
    drop(batcher);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].iter().map(|s| s.items.clone()).collect::<Vec<_>>(), [vec![0, 1], vec![2], vec![3]]);
}
#[test]
fn drains_ready_requests_without_crossing_a_control_boundary() {
    let ctx = Rc::new(());
    let mut queue = VecDeque::from([Message::Apply(segment([1], &ctx)), Message::Boundary, Message::Apply(segment([2], &ctx))]);
    let mut packets = Vec::new();
    let boundary = drain(
        segment([0], &ctx),
        || queue.pop_front(),
        |p| {
            packets.push(p);
            Ok::<_, ()>(())
        },
        Some,
        || true,
    )
    .unwrap();
    assert!(matches!(boundary, Some(Message::Boundary)));
    assert_eq!(queue.len(), 1);
    assert_eq!(packets[0][0].items, [0, 1]);
}
#[test]
fn yields_without_consuming_when_budget_expires() {
    let ctx = Rc::new(());
    let mut queue = VecDeque::from([Message::Apply(segment([1], &ctx))]);
    let mut packets = Vec::new();
    drain(
        segment([0], &ctx),
        || queue.pop_front(),
        |p| {
            packets.push(p);
            Ok::<_, ()>(())
        },
        Some,
        || false,
    )
    .unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(packets[0][0].items, [0]);
}
#[test]
fn flushes_immediately_when_queue_is_empty() {
    let ctx = Rc::new(());
    let mut packets = Vec::new();
    let boundary = drain::<Message, _, _, _, _, ()>(
        segment([0], &ctx),
        || None,
        |p| {
            packets.push(p);
            Ok(())
        },
        Some,
        || true,
    )
    .unwrap();
    assert!(boundary.is_none());
    assert_eq!(packets[0][0].items, [0]);
}
#[test]
fn rejected_requests_are_not_packetized() {
    let ctx = Rc::new(());
    let mut queue = VecDeque::from([Message::Apply(segment([1], &ctx))]);
    let mut packets = Vec::new();
    drain(
        segment([0], &ctx),
        || queue.pop_front(),
        |p| {
            packets.push(p);
            Ok::<_, ()>(())
        },
        |s| if s.items == [0] { None } else { Some(s) },
        || true,
    )
    .unwrap();
    assert_eq!(packets[0][0].items, [1]);
}
#[test]
fn hot_queue_yields_at_request_limit_even_for_empty_requests() {
    let ctx = Rc::new(());
    let mut queue: VecDeque<_> = (0..MAX_REQUESTS + 3).map(|_| Message::Apply(segment([], &ctx))).collect();
    let mut packets = Vec::new();
    drain(
        segment([], &ctx),
        || queue.pop_front(),
        |p| {
            packets.push(p);
            Ok::<_, ()>(())
        },
        Some,
        || true,
    )
    .unwrap();
    assert_eq!(queue.len(), 4);
    assert!(packets.is_empty());
}
#[test]
fn propagates_dispatch_errors() {
    let ctx = Rc::new(());
    let mut batcher = Batcher::new(|_: Vec<Segment>| Err("closed"), 1);
    assert_eq!(batcher.push(segment([0], &ctx)), Err("closed"));
}
#[test]
#[should_panic(expected = "packet limit must be positive")]
fn rejects_zero_packet_limit() {
    let _ = Batcher::new(|_: Vec<Segment>| Ok::<_, ()>(()), 0);
}
