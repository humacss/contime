use std::collections::VecDeque;
use super::{ContimeKey};

#[inline(always)]
pub fn index_before<T, F>(items: &VecDeque<T>, key: ContimeKey, key_func: F) -> Option<usize>
where
    F: Fn(&T) -> ContimeKey,
{
    if items.is_empty() || key < key_func(items.front().unwrap()) {
        return None;
    }

    if key > key_func(items.back().unwrap()) {
        return Some(items.len()-1);
    }

    let time_start_index = items.partition_point(|item| key_func(item) < key);

    time_start_index.checked_sub(1)
}

#[inline(always)]
pub fn indexes_between<T, F>(items: &VecDeque<T>, start_key: ContimeKey, end_key: ContimeKey, key: F) -> (usize, usize)
where
    F: Fn(&T) -> ContimeKey,
{
    if items.is_empty() || start_key >= end_key {
        return (0, 0);
    }

    let start_index = items.partition_point(|item| key(item) < start_key);
    let end_index = items.partition_point(|item| key(item) < end_key);

    (start_index, end_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    type Time = i64;

    #[rstest]
    #[case::empty(vec![], 0, None)]
    #[case::before_none(vec![1], 0, None)]
    #[case::before_single(vec![1], 2, Some(0))]
    #[case::before_exact(vec![5], 5, None)]
    #[case::exact_first(vec![1, 2, 3], 1, None)]
    #[case::exact_middle(vec![1, 2, 3], 2, Some(0))]
    #[case::exact_last(vec![1, 2, 3], 3, Some(1))]
    #[case::duplicates(vec![3, 3], 3, None)]
    #[case::duplicates_before(vec![1, 2, 2, 2, 3], 2, Some(0))]
    #[case::duplicates_middle(vec![1, 2, 2, 2, 3], 3, Some(3))]
    #[case::duplicates_after(vec![1, 2, 2, 2, 3], 4, Some(4))]
    fn test_index_before(#[case] times: Vec<Time>, #[case] query: Time, #[case] expected: Option<usize>) {
        let actual = index_before(&times.into(), ContimeKey { id: 0, time: query }, |&t| ContimeKey { id: 0, time: t });
        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::empty(vec![], 0, 10, vec![])]
    #[case::degenerate_start_ge_end(vec![1,2,3], 5, 5, vec![])]
    #[case::start_gt_end(vec![1,2,3], 10, 5, vec![])]
    #[case::no_overlap_before(vec![5,6,7], 0, 5, vec![])]
    #[case::no_overlap_after(vec![1,2,3], 4, 10, vec![])]
    #[case::full_range(vec![1,3,5,7], 0, 10, vec![1,3,5,7])]
    #[case::exact_start(vec![1,3,5], 3, 10, vec![3,5])]
    #[case::strict_inside(vec![1,3,5,7], 3, 7, vec![3,5])]
    #[case::upper_exclusive(vec![1,3,5,7], 0, 7, vec![1,3,5])] // 7 excluded
    #[case::duplicates_all(vec![4,4,4,4], 4, 5, vec![4,4,4,4])]
    #[case::duplicates_partial(vec![1,4,4,4,7], 4, 7, vec![4,4,4])]
    #[case::duplicates_straddle(vec![1,4,4,5,5,7], 4, 6, vec![4,4,5,5])]
    fn test_indexes_between(#[case] times: Vec<Time>, #[case] start: Time, #[case] end: Time, #[case] expected_times: Vec<Time>) {
        let (left, right) = indexes_between(&times.clone().into(), ContimeKey { id: 0, time: start }, ContimeKey { id: 0, time: end } , |&t| ContimeKey { id: 0, time: t });

        let actual: Vec<Time> = times[left..right].to_vec();
        assert_eq!(actual, expected_times);
    }
}
