use ahash::AHashMap;

use crate::types::{NotificationId, ReplayUpdate, SnapshotSlot};
use crate::SnapshotListener;

struct NotificationCollection<T, L> {
    watched_time: T,
    listener: L,
    pending_snapshot_ids: Vec<u128>,
}

struct CollectionEntry<T, L> {
    generation: u64,
    collection: Option<NotificationCollection<T, L>>,
}

pub(crate) struct NotificationCollections<T, L> {
    entries: Vec<CollectionEntry<T, L>>,
    free: Vec<usize>,
    touched: Vec<NotificationId>,
    active: usize,
}

impl<T, L> NotificationCollections<T, L>
where
    T: Clone + Ord,
    L: SnapshotListener<T>,
{
    pub(crate) const fn new() -> Self {
        Self { entries: Vec::new(), free: Vec::new(), touched: Vec::new(), active: 0 }
    }

    pub(crate) fn register<S, K, C, R>(
        &mut self,
        time: T,
        mut snapshot_ids: Vec<u128>,
        listener: L,
        snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>,
    ) -> Option<NotificationId> {
        snapshot_ids.sort_unstable();
        snapshot_ids.dedup();
        if snapshot_ids.is_empty() || !listener.registered(time.clone(), snapshot_ids.clone()) {
            return None;
        }

        let notification_id = self.insert(NotificationCollection { watched_time: time, listener, pending_snapshot_ids: Vec::new() });
        for snapshot_id in snapshot_ids {
            snapshots.entry(snapshot_id).or_insert_with(SnapshotSlot::metadata_only).notification_ids.push(notification_id);
        }
        Some(notification_id)
    }

    fn insert(&mut self, collection: NotificationCollection<T, L>) -> NotificationId {
        self.active += 1;
        if let Some(index) = self.free.pop() {
            let entry = &mut self.entries[index];
            entry.generation = entry.generation.wrapping_add(1);
            entry.collection = Some(collection);
            NotificationId { index, generation: entry.generation }
        } else {
            let index = self.entries.len();
            self.entries.push(CollectionEntry { generation: 0, collection: Some(collection) });
            NotificationId { index, generation: 0 }
        }
    }

    pub(crate) fn record<S, K, C, R>(&mut self, update: ReplayUpdate<T>, snapshots: &mut AHashMap<u128, SnapshotSlot<S, K, C, R>>) {
        if self.active == 0 {
            return;
        }
        let Some(slot) = snapshots.get_mut(&update.snapshot_id) else { return };
        let mut retained = 0;
        for index in 0..slot.notification_ids.len() {
            let notification_id = slot.notification_ids[index];
            let Some(entry) = self.entries.get_mut(notification_id.index) else { continue };
            if entry.generation != notification_id.generation {
                continue;
            }
            let Some(collection) = entry.collection.as_mut() else { continue };
            slot.notification_ids[retained] = notification_id;
            retained += 1;
            if update.affected_from <= collection.watched_time {
                if collection.pending_snapshot_ids.is_empty() {
                    self.touched.push(notification_id);
                }
                collection.pending_snapshot_ids.push(update.snapshot_id);
            }
        }
        slot.notification_ids.truncate(retained);
    }

    pub(crate) fn flush(&mut self) {
        for notification_id in self.touched.drain(..) {
            let Some(entry) = self.entries.get_mut(notification_id.index) else { continue };
            if entry.generation != notification_id.generation {
                continue;
            }
            let Some(collection) = entry.collection.as_mut() else { continue };
            let snapshot_ids = std::mem::take(&mut collection.pending_snapshot_ids);
            if collection.listener.replayed(collection.watched_time.clone(), snapshot_ids) {
                continue;
            }
            entry.collection = None;
            self.active -= 1;
            self.free.push(notification_id.index);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use ahash::AHashMap;
    use criterion::{BatchSize, Criterion};
    use crossbeam_channel::{unbounded, Sender};

    use super::NotificationCollections;
    use crate::types::{ReplayUpdate, SnapshotSlot};
    use crate::SnapshotListener;

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Message {
        Registered { time: u64, snapshot_ids: Vec<u128> },
        Replayed { time: u64, snapshot_ids: Vec<u128> },
    }

    #[derive(Clone)]
    struct Listener(Sender<Message>);

    impl SnapshotListener<u64> for Listener {
        fn registered(&self, time: u64, snapshot_ids: Vec<u128>) -> bool {
            self.0.send(Message::Registered { time, snapshot_ids }).is_ok()
        }

        fn replayed(&self, time: u64, snapshot_ids: Vec<u128>) -> bool {
            self.0.send(Message::Replayed { time, snapshot_ids }).is_ok()
        }
    }

    type Snapshots = AHashMap<u128, SnapshotSlot<Vec<u8>, Vec<u8>, (), ()>>;

    #[test]
    fn registration_creates_one_collection_on_unique_metadata_only_slots() {
        let (sender, receiver) = unbounded();
        let mut collections = NotificationCollections::new();
        let mut snapshots = Snapshots::new();

        let notification_id = collections.register(55, vec![8, 3, 8, 5], Listener(sender), &mut snapshots).unwrap();

        assert_eq!(receiver.recv().unwrap(), Message::Registered { time: 55, snapshot_ids: vec![3, 5, 8] });
        assert_eq!(snapshots.len(), 3);
        assert!(snapshots.values().all(|slot| slot.events.is_none()));
        assert!(snapshots.values().all(|slot| slot.notification_ids == vec![notification_id]));
    }

    #[test]
    fn one_replay_batch_sends_one_message_with_every_matching_snapshot() {
        let (sender, receiver) = unbounded();
        let mut collections = NotificationCollections::new();
        let mut snapshots = Snapshots::new();
        collections.register(55, vec![3, 5, 8], Listener(sender), &mut snapshots).unwrap();
        receiver.recv().unwrap();

        collections.record(ReplayUpdate { snapshot_id: 3, affected_from: 40 }, &mut snapshots);
        collections.record(ReplayUpdate { snapshot_id: 5, affected_from: 55 }, &mut snapshots);
        collections.record(ReplayUpdate { snapshot_id: 8, affected_from: 56 }, &mut snapshots);
        collections.flush();

        assert_eq!(receiver.recv().unwrap(), Message::Replayed { time: 55, snapshot_ids: vec![3, 5] });
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn reused_collection_indexes_do_not_activate_stale_snapshot_memberships() {
        let (first_sender, first_receiver) = unbounded();
        let mut collections = NotificationCollections::new();
        let mut snapshots = Snapshots::new();
        let first_id = collections.register(10, vec![3], Listener(first_sender), &mut snapshots).unwrap();
        first_receiver.recv().unwrap();
        drop(first_receiver);
        collections.record(ReplayUpdate { snapshot_id: 3, affected_from: 0 }, &mut snapshots);
        collections.flush();

        let (second_sender, second_receiver) = unbounded();
        let second_id = collections.register(10, vec![5], Listener(second_sender), &mut snapshots).unwrap();
        second_receiver.recv().unwrap();
        assert_ne!(first_id, second_id);

        collections.record(ReplayUpdate { snapshot_id: 3, affected_from: 0 }, &mut snapshots);
        collections.flush();
        assert!(second_receiver.try_recv().is_err());
        assert!(snapshots.get(&3).unwrap().notification_ids.is_empty());
    }

    #[test]
    #[ignore = "inline Criterion benchmark"]
    fn benchmark_listeners() {
        let mut criterion = Criterion::default();
        criterion.bench_function("worker/listen/replay/no_listeners", |bencher| {
            let mut collections = NotificationCollections::<u64, Listener>::new();
            let mut snapshots = Snapshots::new();
            bencher.iter(|| {
                collections.record(black_box(ReplayUpdate { snapshot_id: 7, affected_from: 55 }), black_box(&mut snapshots));
            });
        });
        criterion.bench_function("worker/listen/replay/one_nonmatching_collection", |bencher| {
            let (sender, receiver) = unbounded();
            let mut collections = NotificationCollections::new();
            let mut snapshots = Snapshots::new();
            collections.register(54, vec![7], Listener(sender), &mut snapshots);
            receiver.recv().unwrap();
            bencher.iter(|| {
                collections.record(black_box(ReplayUpdate { snapshot_id: 7, affected_from: 55 }), black_box(&mut snapshots));
            });
        });
        criterion.bench_function("worker/listen/register_1000", |bencher| {
            bencher.iter_batched(
                || {
                    let (sender, receiver) = unbounded();
                    (NotificationCollections::new(), Snapshots::new(), Listener(sender), receiver)
                },
                |(mut collections, mut snapshots, listener, receiver)| {
                    collections.register(55, (0..1_000).collect(), listener, &mut snapshots);
                    black_box(receiver.recv().unwrap());
                    black_box((collections, snapshots));
                },
                BatchSize::LargeInput,
            );
        });

        for snapshot_count in [1usize, 100, 1_000] {
            criterion.bench_function(&format!("worker/listen/replay_batch/{snapshot_count}_snapshots"), |bencher| {
                let (sender, receiver) = unbounded();
                let mut collections = NotificationCollections::new();
                let mut snapshots = Snapshots::new();
                collections.register(55, (0..snapshot_count as u128).collect(), Listener(sender), &mut snapshots);
                receiver.recv().unwrap();
                bencher.iter(|| {
                    for snapshot_id in 0..snapshot_count as u128 {
                        collections.record(ReplayUpdate { snapshot_id, affected_from: 55 }, &mut snapshots);
                    }
                    collections.flush();
                    black_box(receiver.recv().unwrap());
                });
            });
        }
        criterion.final_summary();
    }
}
