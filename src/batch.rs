use ahash::AHashMap;

use crate::history::RETAINED_ID_BYTES;
use crate::{EventRejection, EventRejectionReason, Input, InputLanes, SnapshotLanes};

/// Opaque prepared snapshot batch used by doc-hidden benchmark boundary adapters.
#[doc(hidden)]
pub struct SnapshotInputBatch<IL> {
    pub(crate) inputs: Vec<IL>,
    pub(crate) conservative_bytes: u64,
}

/// API-owned request grouped into one complete batch per snapshot history.
#[doc(hidden)]
pub struct PreparedRequest<IL> {
    pub(crate) snapshots: AHashMap<u128, SnapshotInputBatch<IL>>,
    pub(crate) conservative_bytes: u64,
}

impl<IL> SnapshotInputBatch<IL>
where
    IL: Input,
{
    pub(crate) fn unique_input_ids(&self, target: &mut Vec<u128>) {
        target.extend(self.inputs.iter().map(Input::id));
    }
}

pub(crate) fn prepare_inputs_by_snapshot<SL, IL, I>(inputs: I) -> PreparedRequest<IL>
where
    SL: SnapshotLanes<Input = IL>,
    IL: InputLanes<SL>,
    I: IntoIterator<Item = IL>,
{
    let mut snapshots = AHashMap::<u128, SnapshotInputBatch<IL>>::new();
    let mut total_bytes = 0_u64;

    for input in inputs {
        let conservative_bytes = conservative_route_bytes(&input);
        let mut pending_snapshot_id = None;

        input.visit_snapshot_ids(&mut |snapshot_id| {
            if let Some(previous_snapshot_id) = pending_snapshot_id.replace(snapshot_id) {
                push_routed_input(&mut snapshots, previous_snapshot_id, input.clone(), conservative_bytes);
                total_bytes = total_bytes.saturating_add(conservative_bytes);
            }
        });
        if let Some(final_snapshot_id) = pending_snapshot_id {
            push_routed_input(&mut snapshots, final_snapshot_id, input, conservative_bytes);
            total_bytes = total_bytes.saturating_add(conservative_bytes);
        }
    }

    PreparedRequest { snapshots, conservative_bytes: total_bytes }
}

fn conservative_route_bytes<I: Input>(input: &I) -> u64 {
    input.conservative_size().saturating_add(RETAINED_ID_BYTES)
}

pub(crate) fn memory_full_rejections_for_request<IL>(request: &PreparedRequest<IL>) -> Vec<EventRejection>
where
    IL: Input,
{
    let mut input_ids = Vec::new();
    for batch in request.snapshots.values() {
        batch.unique_input_ids(&mut input_ids);
    }
    input_ids.sort_unstable();
    input_ids.dedup();
    input_ids.into_iter().map(|event_id| EventRejection::new(event_id, EventRejectionReason::MemoryFull)).collect()
}

fn push_routed_input<IL>(snapshots: &mut AHashMap<u128, SnapshotInputBatch<IL>>, snapshot_id: u128, input: IL, conservative_bytes: u64) {
    let batch = snapshots.entry(snapshot_id).or_insert_with(|| SnapshotInputBatch { inputs: Vec::new(), conservative_bytes: 0 });
    batch.inputs.push(input);
    batch.conservative_bytes = batch.conservative_bytes.saturating_add(conservative_bytes);
}

/// Test and benchmark access to production API grouping without exposing its internal batch type.
#[doc(hidden)]
pub struct SnapshotBatchBenchmark;

impl SnapshotBatchBenchmark {
    pub fn group<SL, IL, I>(inputs: I) -> AHashMap<u128, Vec<u128>>
    where
        SL: SnapshotLanes<Input = IL>,
        IL: InputLanes<SL>,
        I: IntoIterator<Item = IL>,
    {
        prepare_inputs_by_snapshot::<SL, IL, I>(inputs)
            .snapshots
            .into_iter()
            .map(|(snapshot_id, batch)| {
                let _conservative_bytes = batch.conservative_bytes;
                (snapshot_id, batch.inputs.iter().map(Input::id).collect())
            })
            .collect()
    }

    pub fn total_conservative_bytes<SL, IL, I>(inputs: I) -> u64
    where
        SL: SnapshotLanes<Input = IL>,
        IL: InputLanes<SL>,
        I: IntoIterator<Item = IL>,
    {
        prepare_inputs_by_snapshot::<SL, IL, I>(inputs).conservative_bytes
    }
}
