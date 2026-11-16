#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_roi::records::ROIEstimateStatus;

use super::support::{TestClock, estimate};
use crate::roi_sync::status::{StatusReporter, idle_status};

fn reporter() -> StatusReporter {
    StatusReporter {
        stored: Arc::new(Mutex::new(idle_status())),
        clock: TestClock::at("2026-01-10T00:00:00Z").handle(),
    }
}

#[test]
fn an_estimated_pull_counts_as_estimated() {
    let reporter = reporter();
    reporter.record_estimate(&estimate(ROIEstimateStatus::Estimated, None));
    let status = reporter.snapshot();
    assert_eq!(status.done, 1);
    assert_eq!(status.estimated, 1);
    assert_eq!(status.reused, 0);
    assert_eq!(status.needs_attention, 0);
}

#[test]
fn a_cached_estimated_pull_counts_as_reused() {
    let reporter = reporter();
    reporter.record_estimate(&estimate(ROIEstimateStatus::Estimated, Some(true)));
    let status = reporter.snapshot();
    assert_eq!(status.done, 1);
    assert_eq!(status.estimated, 1);
    assert_eq!(status.reused, 1);
    assert_eq!(status.needs_attention, 0);
}

#[test]
fn a_needs_review_pull_counts_as_needs_attention() {
    let reporter = reporter();
    reporter.record_estimate(&estimate(ROIEstimateStatus::NeedsReview, None));
    let status = reporter.snapshot();
    assert_eq!(status.done, 1);
    assert_eq!(status.estimated, 0);
    assert_eq!(status.reused, 0);
    assert_eq!(status.needs_attention, 1);
}

#[test]
fn an_error_pull_counts_as_needs_attention() {
    let reporter = reporter();
    reporter.record_estimate(&estimate(ROIEstimateStatus::Error, Some(false)));
    let status = reporter.snapshot();
    assert_eq!(status.done, 1);
    assert_eq!(status.estimated, 0);
    assert_eq!(status.reused, 0);
    assert_eq!(status.needs_attention, 1);
}

#[test]
fn a_cached_needs_review_pull_counts_in_both_counters() {
    let reporter = reporter();
    reporter.record_estimate(&estimate(ROIEstimateStatus::NeedsReview, Some(true)));
    let status = reporter.snapshot();
    assert_eq!(status.done, 1);
    assert_eq!(status.estimated, 0);
    assert_eq!(status.reused, 1);
    assert_eq!(status.needs_attention, 1);
}

#[test]
fn counters_accumulate_across_pulls() {
    let reporter = reporter();
    reporter.record_estimate(&estimate(ROIEstimateStatus::Estimated, Some(false)));
    reporter.record_estimate(&estimate(ROIEstimateStatus::Estimated, Some(true)));
    reporter.record_estimate(&estimate(ROIEstimateStatus::NeedsReview, None));
    let status = reporter.snapshot();
    assert_eq!(status.done, 3);
    assert_eq!(status.estimated, 2);
    assert_eq!(status.reused, 1);
    assert_eq!(status.needs_attention, 1);
}
