#![cfg(test)]

use betterlite_roi::records::ROIEstimateStatus;

use crate::roi_cache::records::cached_record;

use super::support::record;

#[test]
fn the_estimate_is_marked_cached_and_everything_else_is_kept() {
    let pull = record(ROIEstimateStatus::Estimated);
    let cached = cached_record(&pull);
    let mut expected = pull.clone();
    expected.estimate.cached = Some(true);
    assert_eq!(cached, expected);
    assert_eq!(pull.estimate.cached, Some(false));
}

#[test]
fn an_estimate_without_a_cached_flag_gets_one() {
    let mut pull = record(ROIEstimateStatus::Estimated);
    pull.estimate.cached = None;
    assert_eq!(cached_record(&pull).estimate.cached, Some(true));
}

#[test]
fn missing_commit_emails_become_an_empty_list() {
    let mut pull = record(ROIEstimateStatus::Estimated);
    pull.commit_emails = None;
    assert_eq!(cached_record(&pull).commit_emails, Some(Vec::new()));
}

#[test]
fn a_missing_cache_key_stays_missing() {
    let mut pull = record(ROIEstimateStatus::Estimated);
    pull.cache_key = None;
    assert_eq!(cached_record(&pull).cache_key, None);
}
