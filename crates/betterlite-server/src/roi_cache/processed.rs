use std::collections::BTreeMap;

use betterlite_roi::records::{ROIEstimateStatus, ROIPullRecord};

use super::error::ReportBlocked;

pub struct ProcessedPull {
    pub position: usize,
    pub record: ROIPullRecord,
    pub metadata_unavailable: bool,
}

pub fn processed_records(
    processed: Vec<ProcessedPull>,
) -> Result<BTreeMap<usize, ROIPullRecord>, ReportBlocked> {
    if !processed.is_empty() && processed.iter().all(|item| item.metadata_unavailable) {
        return Err(ReportBlocked::NoMetadata);
    }
    let any_error = processed
        .iter()
        .any(|item| item.record.estimate.status == ROIEstimateStatus::Error);
    let any_estimated = processed
        .iter()
        .any(|item| item.record.estimate.status == ROIEstimateStatus::Estimated);
    if any_error && !any_estimated {
        return Err(ReportBlocked::NoEstimates);
    }
    Ok(processed
        .into_iter()
        .map(|item| (item.position, item.record))
        .collect())
}
