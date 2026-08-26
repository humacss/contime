use std::collections::{btree_map, vec_deque, BTreeMap, VecDeque};
use std::iter::Peekable;
use std::ops::Bound;

use ahash::AHashSet;

use crate::{ContimeKey, ContimeTime, Input};

pub(crate) const RETAINED_ID_BYTES: u64 = (size_of::<u128>() * 2) as u64;

#[derive(Debug, Clone)]
pub struct HistoryInputs<T, I>
where
    T: ContimeTime,
    I: Input<Time = T>,
{
    ordered: VecDeque<(ContimeKey<T>, I)>,
    late: BTreeMap<ContimeKey<T>, I>,
    retained_ids: AHashSet<u128>,
}

impl<T, I> Default for HistoryInputs<T, I>
where
    T: ContimeTime,
    I: Input<Time = T>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T, I> HistoryInputs<T, I>
where
    T: ContimeTime,
    I: Input<Time = T>,
{
    pub fn new() -> Self {
        Self { ordered: VecDeque::new(), late: BTreeMap::new(), retained_ids: AHashSet::new() }
    }

    pub fn len(&self) -> usize {
        self.ordered.len() + self.late.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty() && self.late.is_empty()
    }

    pub fn storage_counts(&self) -> (usize, usize) {
        (self.ordered.len(), self.late.len())
    }

    pub fn latest_entry_key(&self) -> Option<(T, u128)> {
        self.latest_key().map(|key| (key.time, key.id))
    }

    pub fn entries(&self) -> impl Iterator<Item = (T, u128, &I)> {
        self.iter().map(|(key, input)| (key.time.clone(), key.id, input))
    }

    pub fn insert_batch(&mut self, inputs: Vec<I>) -> HistoryInsert<T> {
        self.insert_batch_with(inputs, |_| {})
    }

    pub fn prune_before_time(&mut self, time: T) -> (usize, u64) {
        let pruned = self.prune_before(&ContimeKey { time, id: u128::MIN });
        (pruned.count(), pruned.bytes())
    }

    pub(crate) fn insert_batch_with<F>(&mut self, inputs: Vec<I>, mut on_insert: F) -> HistoryInsert<T>
    where
        F: FnMut(&I),
    {
        self.insert_batch_filter_with(inputs, |_| true, &mut on_insert)
    }

    pub(crate) fn insert_batch_filter_with<A, F>(&mut self, inputs: Vec<I>, mut accept: A, mut on_insert: F) -> HistoryInsert<T>
    where
        A: FnMut(&I) -> bool,
        F: FnMut(&I),
    {
        let latest_key_before = self.latest_key();
        let mut keyed_inputs = Vec::with_capacity(inputs.len());
        for input in inputs {
            let input_id = input.id();
            if self.retained_ids.contains(&input_id) || !accept(&input) {
                continue;
            }
            assert!(self.retained_ids.insert(input_id), "history input ID was inserted twice");
            keyed_inputs.push((ContimeKey::from_input(&input), input));
        }
        let fast_path = keyed_inputs
            .first()
            .is_none_or(|(first_key, _input)| latest_key_before.as_ref().is_none_or(|latest_key| first_key > latest_key))
            && keyed_inputs.windows(2).all(|pair| pair[0].0 < pair[1].0);

        if !fast_path {
            keyed_inputs.sort_unstable_by(|(left, _left_input), (right, _right_input)| left.cmp(right));
        }

        let mut inserted = HistoryInsert::new(latest_key_before.clone());
        for (key, input) in keyed_inputs {
            on_insert(&input);
            inserted.record(&key, input.conservative_size().saturating_add(RETAINED_ID_BYTES));
            if latest_key_before.as_ref().is_some_and(|latest_key| key < *latest_key) {
                let previous = self.late.insert(key, input);
                debug_assert!(previous.is_none(), "a late history key was inserted twice");
            } else {
                self.ordered.push_back((key, input));
            }
        }

        self.assert_invariants();
        inserted
    }

    pub(crate) fn iter(&self) -> MergedInputs<'_, T, I> {
        self.range((Bound::Unbounded, Bound::Unbounded))
    }

    pub(crate) fn range(&self, bounds: (Bound<ContimeKey<T>>, Bound<ContimeKey<T>>)) -> MergedInputs<'_, T, I> {
        let (start, end) = bounds;
        let ordered_start = match &start {
            Bound::Included(boundary) => self.ordered.partition_point(|(key, _input)| key < boundary),
            Bound::Excluded(boundary) => self.ordered.partition_point(|(key, _input)| key <= boundary),
            Bound::Unbounded => 0,
        };
        let ordered_end = match &end {
            Bound::Included(boundary) => self.ordered.partition_point(|(key, _input)| key <= boundary),
            Bound::Excluded(boundary) => self.ordered.partition_point(|(key, _input)| key < boundary),
            Bound::Unbounded => self.ordered.len(),
        };
        let late_start = as_ref_bound(&start);
        let late_end = as_ref_bound(&end);

        MergedInputs {
            ordered: self.ordered.range(ordered_start..ordered_end).peekable(),
            late: self.late.range((late_start, late_end)).peekable(),
        }
    }

    pub(crate) fn latest_key(&self) -> Option<ContimeKey<T>> {
        max_key(self.ordered.back().map(|(key, _input)| key), self.late.last_key_value().map(|(key, _input)| key)).cloned()
    }

    pub(crate) fn latest_key_before(&self, boundary: &ContimeKey<T>) -> Option<ContimeKey<T>> {
        let ordered_index = self.ordered.partition_point(|(key, _input)| key < boundary);
        let ordered = ordered_index.checked_sub(1).map(|index| &self.ordered[index].0);
        let late = self.late.range(..boundary).next_back().map(|(key, _input)| key);
        max_key(ordered, late).cloned()
    }

    pub(crate) fn prune_before(&mut self, boundary: &ContimeKey<T>) -> PrunedInputs {
        let mut pruned = PrunedInputs::default();
        while self.ordered.front().is_some_and(|(key, _input)| key < boundary) {
            let (_key, input) = self.ordered.pop_front().expect("the ordered history front was just inspected");
            pruned.record(&input, &mut self.retained_ids);
        }

        let retained = self.late.split_off(boundary);
        let removed = std::mem::replace(&mut self.late, retained);
        for input in removed.into_values() {
            pruned.record(&input, &mut self.retained_ids);
        }

        self.assert_invariants();
        pruned
    }

    #[cfg(debug_assertions)]
    fn assert_invariants(&self) {
        assert!(self.ordered.iter().zip(self.ordered.iter().skip(1)).all(|(left, right)| left.0 < right.0));
        if let Some((ordered_tail, _input)) = self.ordered.back() {
            assert!(self.late.keys().all(|late_key| late_key < ordered_tail));
        }
        assert_eq!(self.len(), self.iter().count());
        assert_eq!(self.len(), self.retained_ids.len());
    }

    #[cfg(not(debug_assertions))]
    #[inline(always)]
    fn assert_invariants(&self) {}
}

fn as_ref_bound<T>(bound: &Bound<T>) -> Bound<&T> {
    match bound {
        Bound::Included(value) => Bound::Included(value),
        Bound::Excluded(value) => Bound::Excluded(value),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn max_key<'a, T: ContimeTime>(left: Option<&'a ContimeKey<T>>, right: Option<&'a ContimeKey<T>>) -> Option<&'a ContimeKey<T>> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(key), None) | (None, Some(key)) => Some(key),
        (None, None) => None,
    }
}

pub(crate) struct MergedInputs<'a, T, I>
where
    T: ContimeTime,
    I: Input<Time = T>,
{
    ordered: Peekable<vec_deque::Iter<'a, (ContimeKey<T>, I)>>,
    late: Peekable<btree_map::Range<'a, ContimeKey<T>, I>>,
}

impl<'a, T, I> Iterator for MergedInputs<'a, T, I>
where
    T: ContimeTime,
    I: Input<Time = T>,
{
    type Item = (&'a ContimeKey<T>, &'a I);

    fn next(&mut self) -> Option<Self::Item> {
        match (self.ordered.peek(), self.late.peek()) {
            (Some((ordered_key, _ordered_input)), Some((late_key, _late_input))) if ordered_key == *late_key => {
                debug_assert!(false, "one history key exists in both hybrid stores");
                let _ = self.late.next();
                self.ordered.next().map(|(key, input)| (key, input))
            }
            (Some((ordered_key, _ordered_input)), Some((late_key, _late_input))) if ordered_key < *late_key => {
                self.ordered.next().map(|(key, input)| (key, input))
            }
            (Some(_), Some(_)) | (None, Some(_)) => self.late.next(),
            (Some(_), None) => self.ordered.next().map(|(key, input)| (key, input)),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HistoryInsert<T: ContimeTime> {
    pub(crate) inserted_count: usize,
    pub(crate) bytes_delta: i64,
    pub(crate) earliest_time: Option<T>,
    pub(crate) latest_key_before: Option<ContimeKey<T>>,
    pub(crate) single_key: Option<ContimeKey<T>>,
}

impl<T: ContimeTime> HistoryInsert<T> {
    fn new(latest_key_before: Option<ContimeKey<T>>) -> Self {
        Self { inserted_count: 0, bytes_delta: 0, earliest_time: None, latest_key_before, single_key: None }
    }

    pub fn inserted_count(&self) -> usize {
        self.inserted_count
    }

    pub fn latest_key_before(&self) -> Option<(T, u128)> {
        self.latest_key_before.as_ref().map(|key| (key.time.clone(), key.id))
    }

    fn record(&mut self, key: &ContimeKey<T>, bytes: u64) {
        self.earliest_time = Some(match self.earliest_time.take() {
            Some(earliest) => earliest.min(key.time.clone()),
            None => key.time.clone(),
        });
        self.bytes_delta = self.bytes_delta.saturating_add(bytes as i64);
        self.inserted_count += 1;
        self.single_key = (self.inserted_count == 1).then(|| key.clone());
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PrunedInputs {
    count: usize,
    bytes: u64,
}

impl PrunedInputs {
    pub(crate) fn count(self) -> usize {
        self.count
    }

    pub(crate) fn bytes(self) -> u64 {
        self.bytes
    }

    fn record<I: Input>(&mut self, input: &I, retained_ids: &mut AHashSet<u128>) {
        assert!(retained_ids.remove(&input.id()), "pruned history input ID was not retained");
        self.count += 1;
        self.bytes = self.bytes.saturating_add(input.conservative_size()).saturating_add(RETAINED_ID_BYTES);
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Bound;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::{ContimeKey, Input};

    use super::HistoryInputs;

    #[derive(Debug)]
    struct StoredTestInput {
        id: u128,
        time: i64,
        drops: Option<Arc<AtomicUsize>>,
    }

    impl Drop for StoredTestInput {
        fn drop(&mut self) {
            if let Some(drops) = &self.drops {
                drops.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    impl Input for StoredTestInput {
        type Time = i64;

        fn id(&self) -> u128 {
            self.id
        }

        fn time(&self) -> Self::Time {
            self.time
        }

        fn conservative_size(&self) -> u64 {
            32
        }
    }

    fn event(id: u128, time: i64) -> StoredTestInput {
        StoredTestInput { id, time, drops: None }
    }

    fn tracked_event(id: u128, time: i64, drops: &Arc<AtomicUsize>) -> StoredTestInput {
        StoredTestInput { id, time, drops: Some(Arc::clone(drops)) }
    }

    fn keys(inputs: &HistoryInputs<i64, StoredTestInput>) -> Vec<(i64, u128)> {
        inputs.iter().map(|(key, _input)| (key.time, key.id)).collect()
    }

    #[test]
    fn ordered_batches_stay_in_the_append_deque() {
        let mut inputs = HistoryInputs::new();

        let inserted = inputs.insert_batch(vec![event(1, 10), event(2, 20), event(3, 30)]);

        assert_eq!(inserted.inserted_count(), 3);
        assert_eq!(inputs.storage_counts(), (3, 0));
        assert_eq!(keys(&inputs), vec![(10, 1), (20, 2), (30, 3)]);
    }

    #[test]
    fn a_middle_input_uses_the_late_tree_and_merges_canonically() {
        let mut inputs = HistoryInputs::new();
        inputs.insert_batch(vec![event(1, 10), event(3, 30)]);

        inputs.insert_batch(vec![event(2, 20)]);

        assert_eq!(inputs.storage_counts(), (2, 1));
        assert_eq!(keys(&inputs), vec![(10, 1), (20, 2), (30, 3)]);
    }

    #[test]
    fn same_time_inputs_are_merged_by_id() {
        let mut inputs = HistoryInputs::new();

        inputs.insert_batch(vec![event(30, 10), event(10, 10), event(20, 10)]);

        assert_eq!(inputs.storage_counts(), (3, 0));
        assert_eq!(keys(&inputs), vec![(10, 10), (10, 20), (10, 30)]);
    }

    #[test]
    fn an_exact_duplicate_key_is_not_inserted_again() {
        let mut inputs = HistoryInputs::new();
        inputs.insert_batch(vec![event(1, 10), event(2, 20)]);

        let inserted = inputs.insert_batch(vec![event(1, 10)]);

        assert_eq!(inserted.inserted_count(), 0);
        assert_eq!(inputs.storage_counts(), (2, 0));
        assert_eq!(keys(&inputs), vec![(10, 1), (20, 2)]);
    }

    #[test]
    fn merged_ranges_honor_inclusive_and_exclusive_bounds() {
        let mut inputs = HistoryInputs::new();
        inputs.insert_batch(vec![event(1, 10), event(3, 30), event(5, 50)]);
        inputs.insert_batch(vec![event(2, 20), event(4, 40)]);

        let ranged = inputs
            .range((Bound::Excluded(ContimeKey { time: 20, id: 2 }), Bound::Included(ContimeKey { time: 40, id: 4 })))
            .map(|(key, _input)| (key.time, key.id))
            .collect::<Vec<_>>();

        assert_eq!(ranged, vec![(30, 3), (40, 4)]);
    }

    #[test]
    fn latest_key_before_compares_both_stores() {
        let mut inputs = HistoryInputs::new();
        inputs.insert_batch(vec![event(1, 10), event(3, 30), event(5, 50)]);
        inputs.insert_batch(vec![event(2, 20), event(4, 40)]);

        assert_eq!(inputs.latest_key_before(&ContimeKey { time: 45, id: u128::MIN }), Some(ContimeKey { time: 40, id: 4 }));
        assert_eq!(inputs.latest_key(), Some(ContimeKey { time: 50, id: 5 }));
    }

    #[test]
    fn pruning_drops_ordered_and_late_payloads() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut inputs = HistoryInputs::new();
        inputs.insert_batch(vec![tracked_event(1, 10, &drops), tracked_event(3, 30, &drops)]);
        inputs.insert_batch(vec![tracked_event(2, 20, &drops)]);

        let pruned = inputs.prune_before(&ContimeKey { time: 25, id: u128::MIN });

        assert_eq!(pruned.count(), 2);
        assert_eq!(pruned.bytes(), 128);
        assert_eq!(drops.load(Ordering::Relaxed), 2);
        assert_eq!(keys(&inputs), vec![(30, 3)]);
    }
}
