use ahash::AHashMap;

use crate::history::RETAINED_ID_BYTES;
use crate::{EventRejection, EventRejectionReason, Input, InputLanes, SnapshotLanes};

/// Opaque prepared snapshot batch used by doc-hidden benchmark boundary adapters.
#[doc(hidden)]
pub struct SnapshotInputBatch<IL> {
    pub(crate) snapshot_id: u128,
    pub(crate) inputs: Vec<IL>,
    pub(crate) conservative_bytes: u64,
    snapshot_materialization_accounted: bool,
}

impl<IL> SnapshotInputBatch<IL>
where
    IL: Input,
{
    pub(crate) fn unique_input_ids(&self, target: &mut Vec<u128>) {
        target.extend(self.inputs.iter().map(Input::id));
    }
}

pub(crate) fn group_inputs_by_snapshot<SL, IL, I>(inputs: I) -> Vec<SnapshotInputBatch<IL>>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    I: IntoIterator<Item = IL>,
{
    let mut batch_index_by_snapshot = AHashMap::<u128, usize>::new();
    let mut batches = Vec::<SnapshotInputBatch<IL>>::new();
    let mut routed_snapshot_ids = Vec::<u128>::new();

    for input in inputs {
        routed_snapshot_ids.clear();
        input.visit_snapshot_ids(&mut |snapshot_id| routed_snapshot_ids.push(snapshot_id));
        let Some((&final_snapshot_id, earlier_snapshot_ids)) = routed_snapshot_ids.split_last() else {
            continue;
        };
        let conservative_bytes = conservative_route_bytes::<SL, IL>(&input);

        for &snapshot_id in earlier_snapshot_ids {
            push_routed_input::<SL, IL>(&mut batches, &mut batch_index_by_snapshot, snapshot_id, input.clone(), conservative_bytes);
        }
        push_routed_input::<SL, IL>(&mut batches, &mut batch_index_by_snapshot, final_snapshot_id, input, conservative_bytes);
    }

    batches
}

fn conservative_route_bytes<SL, IL>(input: &IL) -> u64
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
{
    input.conservative_size().saturating_add(input.conservative_allocation_size()).saturating_add(RETAINED_ID_BYTES)
}

pub(crate) fn total_conservative_bytes<IL>(batches: &[SnapshotInputBatch<IL>]) -> u64 {
    batches.iter().fold(0, |total, batch| total.saturating_add(batch.conservative_bytes))
}

pub(crate) fn memory_full_rejections<IL>(batches: &[SnapshotInputBatch<IL>]) -> Vec<EventRejection>
where
    IL: Input,
{
    let mut input_ids = Vec::new();
    for batch in batches {
        batch.unique_input_ids(&mut input_ids);
    }
    input_ids.sort_unstable();
    input_ids.dedup();
    input_ids.into_iter().map(|event_id| EventRejection::new(event_id, EventRejectionReason::MemoryFull)).collect()
}

fn push_routed_input<SL, IL>(
    batches: &mut Vec<SnapshotInputBatch<IL>>,
    batch_index_by_snapshot: &mut AHashMap<u128, usize>,
    snapshot_id: u128,
    input: IL,
    conservative_bytes: u64,
) where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
{
    let batch_index = *batch_index_by_snapshot.entry(snapshot_id).or_insert_with(|| {
        let batch_index = batches.len();
        batches.push(SnapshotInputBatch {
            snapshot_id,
            inputs: Vec::new(),
            conservative_bytes: 0,
            snapshot_materialization_accounted: false,
        });
        batch_index
    });
    let batch = &mut batches[batch_index];
    if !batch.snapshot_materialization_accounted {
        if let Some(snapshot) = SL::materialize(snapshot_id, &input) {
            batch.conservative_bytes =
                batch.conservative_bytes.saturating_add(snapshot.conservative_size()).saturating_add(std::mem::size_of::<u64>() as u64);
            batch.snapshot_materialization_accounted = true;
        }
    }
    batch.inputs.push(input);
    batch.conservative_bytes = batch.conservative_bytes.saturating_add(conservative_bytes);
}

/// Test and benchmark access to production API grouping without exposing its internal batch type.
#[doc(hidden)]
pub struct SnapshotBatchBenchmark;

impl SnapshotBatchBenchmark {
    pub fn group<SL, IL, I>(inputs: I) -> Vec<(u128, Vec<u128>)>
    where
        SL: SnapshotLanes<Input = IL>,
        IL: InputLanes<SL>,
        I: IntoIterator<Item = IL>,
    {
        group_inputs_by_snapshot::<SL, IL, I>(inputs)
            .into_iter()
            .map(|batch| {
                let _conservative_bytes = batch.conservative_bytes;
                (batch.snapshot_id, batch.inputs.iter().map(Input::id).collect())
            })
            .collect()
    }

    pub fn total_conservative_bytes<SL, IL, I>(inputs: I) -> u64
    where
        SL: SnapshotLanes<Input = IL>,
        IL: InputLanes<SL>,
        I: IntoIterator<Item = IL>,
    {
        let batches = group_inputs_by_snapshot::<SL, IL, I>(inputs);
        total_conservative_bytes(&batches)
    }
}
