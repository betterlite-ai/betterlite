use crate::analysis::primitives::partition_items;
use crate::analysis::{AnalysisError, Observation, ObservationKind};

pub(super) fn observation_batches(
    observations: &[Observation],
) -> Result<Vec<Vec<Observation>>, AnalysisError> {
    let mut ordered = observations.to_vec();
    ordered.sort_by_key(|observation| (observation.check_id.clone(), kind_order(observation.kind)));
    partition_items(
        &ordered,
        |observation| serde_json::to_vec(observation).map_or(usize::MAX, |bytes| bytes.len()),
        16000,
    )
}

pub(super) const fn kind_order(kind: ObservationKind) -> u8 {
    match kind {
        ObservationKind::Issue => 0,
        ObservationKind::Pattern => 1,
    }
}
