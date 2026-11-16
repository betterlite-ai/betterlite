#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;

use crate::roi_cache::processed::{ProcessedPull, processed_records};

use super::support::{processed, record};

#[test]
fn each_position_maps_to_its_record() {
    let mut second = processed(5, ROIEstimateStatus::Estimated, false);
    second.record.number = 99;
    let batch = vec![
        processed(2, ROIEstimateStatus::Estimated, false),
        second,
        processed(0, ROIEstimateStatus::NeedsReview, false),
    ];
    let records = processed_records(batch).unwrap();
    assert_eq!(
        records.keys().copied().collect::<Vec<usize>>(),
        vec![0, 2, 5]
    );
    assert_eq!(
        records.get(&0),
        Some(&record(ROIEstimateStatus::NeedsReview))
    );
    assert_eq!(records.get(&2), Some(&record(ROIEstimateStatus::Estimated)));
    assert_eq!(records.get(&5).map(|found| found.number), Some(99));
}

#[test]
fn a_repeated_position_keeps_the_last_record() {
    let mut last = record(ROIEstimateStatus::Estimated);
    last.number = 42;
    let batch = vec![
        processed(1, ROIEstimateStatus::Estimated, false),
        ProcessedPull {
            position: 1,
            record: last,
            metadata_unavailable: false,
        },
    ];
    let records = processed_records(batch).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records.get(&1).map(|found| found.number), Some(42));
}
