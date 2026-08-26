use ahash::AHashMap;

use crate::history::RETAINED_ID_BYTES;
use crate::{Input, InputLanes, SnapshotLanes};

pub(crate) struct SnapshotInputBatch<IL> {
    pub(crate) snapshot_id: u128,
    pub(crate) inputs: Vec<IL>,
    pub(crate) conservative_bytes: u64,
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
        let conservative_bytes = conservative_route_bytes(&input);

        for &snapshot_id in earlier_snapshot_ids {
            push_routed_input(&mut batches, &mut batch_index_by_snapshot, snapshot_id, input.clone(), conservative_bytes);
        }
        push_routed_input(&mut batches, &mut batch_index_by_snapshot, final_snapshot_id, input, conservative_bytes);
    }

    batches
}

fn conservative_route_bytes<I: Input>(input: &I) -> u64 {
    input.conservative_size().saturating_mul(2).saturating_add(RETAINED_ID_BYTES)
}

fn push_routed_input<IL>(
    batches: &mut Vec<SnapshotInputBatch<IL>>,
    batch_index_by_snapshot: &mut AHashMap<u128, usize>,
    snapshot_id: u128,
    input: IL,
    conservative_bytes: u64,
) {
    let batch_index = *batch_index_by_snapshot.entry(snapshot_id).or_insert_with(|| {
        let batch_index = batches.len();
        batches.push(SnapshotInputBatch { snapshot_id, inputs: Vec::new(), conservative_bytes: 0 });
        batch_index
    });
    let batch = &mut batches[batch_index];
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
}
