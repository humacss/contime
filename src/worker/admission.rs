use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use ahash::AHashMap;

use super::WorkerInput;
use crate::{ContimeTime, EventRejection, EventRejectionReason, Input};

pub(super) struct AdmissionResult<IL> {
    pub(super) inputs: Vec<WorkerInput<IL>>,
    pub(super) rejections: Vec<EventRejection>,
    pub(super) reserved_bytes: u64,
    pub(super) identity_bytes: u64,
}

pub(super) struct WorkerAdmission<T> {
    retained_ids: HashSet<u128>,
    ids_by_retention_time: BTreeMap<T, Vec<u128>>,
    current_time: T,
    horizon_delta: T,
}

impl<T> WorkerAdmission<T>
where
    T: ContimeTime,
{
    pub(super) fn new(horizon_delta: T) -> Self {
        Self { retained_ids: HashSet::new(), ids_by_retention_time: BTreeMap::new(), current_time: T::default(), horizon_delta }
    }

    pub(super) fn admit<IL>(
        &mut self,
        inputs: Vec<WorkerInput<IL>>,
        memory_budget: &AtomicU64,
        memory_usage: &AtomicU64,
    ) -> AdmissionResult<IL>
    where
        IL: Input<Time = T>,
    {
        let mut group_by_id = AHashMap::<u128, usize>::new();
        let mut groups = Vec::<Vec<WorkerInput<IL>>>::new();
        for routed_input in inputs {
            let input_id = routed_input.input.id();
            if let Some(&index) = group_by_id.get(&input_id) {
                groups[index].push(routed_input);
            } else {
                group_by_id.insert(input_id, groups.len());
                groups.push(vec![routed_input]);
            }
        }

        let earliest_time = self.current_time.clone().saturating_sub(self.horizon_delta.clone());
        let mut admitted = Vec::new();
        let mut rejections = Vec::new();
        let mut reserved_bytes = 0u64;
        let mut identity_bytes = 0u64;

        for mut group in groups {
            let first = &group[0].input;
            let input_id = first.id();
            if self.retained_ids.contains(&input_id) {
                continue;
            }

            let input_time = first.time();
            if input_time < earliest_time {
                rejections.push(EventRejection::new(input_id, EventRejectionReason::BeforeHistoryHorizon));
                continue;
            }

            let identity_size = identity_entry_size::<T>();
            let lane_size = first.conservative_size();
            let estimate = identity_size
                .saturating_add(lane_size.saturating_mul(2))
                .saturating_add((group.len() as u64).saturating_mul(size_of::<u128>() as u64));
            if !try_reserve(memory_usage, memory_budget.load(Ordering::Relaxed), estimate) {
                rejections.push(EventRejection::new(input_id, EventRejectionReason::MemoryFull));
                continue;
            }

            assert!(self.retained_ids.insert(input_id), "worker input ID was inserted twice");
            self.ids_by_retention_time.entry(input_time).or_default().push(input_id);
            reserved_bytes = reserved_bytes.saturating_add(estimate);
            identity_bytes = identity_bytes.saturating_add(identity_size);
            admitted.append(&mut group);
        }

        AdmissionResult { inputs: admitted, rejections, reserved_bytes, identity_bytes }
    }

    pub(super) fn advance_to(&mut self, time: T) -> u64 {
        self.current_time = time.clone();
        let earliest_time = time.saturating_sub(self.horizon_delta.clone());
        let retained = self.ids_by_retention_time.split_off(&earliest_time);
        let removed = std::mem::replace(&mut self.ids_by_retention_time, retained);
        let removed_count = removed.into_values().flatten().filter(|input_id| self.retained_ids.remove(input_id)).count() as u64;
        removed_count.saturating_mul(identity_entry_size::<T>())
    }
}

fn try_reserve(memory_usage: &AtomicU64, budget: u64, bytes: u64) -> bool {
    memory_usage.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(bytes).filter(|next| *next < budget)).is_ok()
}

const fn identity_entry_size<T>() -> u64 {
    (size_of::<u128>() * 2 + size_of::<T>()) as u64
}
