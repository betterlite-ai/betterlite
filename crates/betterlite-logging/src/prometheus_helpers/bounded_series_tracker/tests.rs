#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric, SeriesKey, SeriesLimits,
    TrackerError,
};

#[derive(Debug, thiserror::Error)]
#[error("the probe metric refused the removal")]
struct ProbeError;

#[derive(Clone, Copy, Debug)]
enum ProbeOutcome {
    Removed,
    Absent,
    Unsupported,
    Failed,
}

#[derive(Clone, Debug)]
struct ProbeMetric {
    outcome: ProbeOutcome,
    attempts: Arc<Mutex<Vec<Vec<Option<String>>>>>,
}

impl ProbeMetric {
    fn new(outcome: ProbeOutcome) -> Self {
        Self {
            outcome,
            attempts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn attempts(&self) -> Vec<Vec<Option<String>>> {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RemovableMetric for ProbeMetric {
    type Error = ProbeError;

    fn remove_child(&self, label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(label_values.to_vec());
        match self.outcome {
            ProbeOutcome::Removed => Ok(ChildRemoval::Removed),
            ProbeOutcome::Absent => Ok(ChildRemoval::Absent),
            ProbeOutcome::Unsupported => Ok(ChildRemoval::Unsupported),
            ProbeOutcome::Failed => Err(ProbeError),
        }
    }
}

fn values(items: &[&str]) -> Vec<Option<String>> {
    items.iter().copied().map(str::to_owned).map(Some).collect()
}

fn key<'series>(
    metric_name: &'series str,
    label_values: &'series [Option<String>],
) -> SeriesKey<'series> {
    SeriesKey {
        metric_name,
        label_values,
    }
}

fn limits(
    max_series: Option<i64>,
    ttl_seconds: Option<u64>,
    cleanup_seconds: Option<u64>,
) -> SeriesLimits {
    SeriesLimits {
        max_series,
        ttl: ttl_seconds.map(Duration::from_secs),
        cleanup_interval: cleanup_seconds.map(Duration::from_secs),
    }
}

fn at(start: Instant, seconds: u64) -> Instant {
    start + Duration::from_secs(seconds)
}

#[test]
fn evicts_the_oldest_series_when_a_new_one_passes_the_limit() {
    let metric = ProbeMetric::new(ProbeOutcome::Removed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    let bounds = limits(Some(2), None, None);
    tracker
        .track_series(&metric, key("m", &first), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &second), bounds, start)
        .unwrap();
    assert_eq!(metric.attempts(), Vec::new());
    tracker
        .track_series(&metric, key("m", &third), bounds, start)
        .unwrap();
    assert_eq!(metric.attempts(), vec![first]);
}

#[test]
fn moves_a_repeated_series_to_the_end_of_the_eviction_order() {
    let metric = ProbeMetric::new(ProbeOutcome::Removed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    let bounds = limits(Some(2), None, None);
    tracker
        .track_series(&metric, key("m", &first), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &second), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &first), bounds, at(start, 1))
        .unwrap();
    tracker
        .track_series(&metric, key("m", &third), bounds, at(start, 1))
        .unwrap();
    assert_eq!(metric.attempts(), vec![second]);
}

#[test]
fn treats_zero_or_negative_max_series_as_unlimited() {
    let zeroed = ProbeMetric::new(ProbeOutcome::Removed);
    let negative = ProbeMetric::new(ProbeOutcome::Removed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    let bounds = [
        (&zeroed, limits(Some(0), None, None)),
        (&negative, limits(Some(-5), None, None)),
    ];
    for (probe, step_limits) in bounds {
        tracker
            .track_series(probe, key("m", &first), step_limits, start)
            .unwrap();
        tracker
            .track_series(probe, key("m", &second), step_limits, start)
            .unwrap();
        tracker
            .track_series(probe, key("m", &third), step_limits, start)
            .unwrap();
    }
    assert_eq!(zeroed.attempts(), Vec::new());
    assert_eq!(negative.attempts(), Vec::new());
}

#[test]
fn stops_evicting_when_the_metric_refuses_a_removal() {
    let metric = ProbeMetric::new(ProbeOutcome::Unsupported);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    let bounds = limits(Some(1), None, None);
    tracker
        .track_series(&metric, key("m", &first), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &second), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &third), bounds, start)
        .unwrap();
    assert_eq!(metric.attempts(), vec![first.clone(), first]);
}

#[test]
fn drops_a_series_that_the_metric_reports_absent() {
    let metric = ProbeMetric::new(ProbeOutcome::Absent);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    let bounds = limits(Some(1), None, None);
    tracker
        .track_series(&metric, key("m", &first), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &second), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &third), bounds, start)
        .unwrap();
    assert_eq!(metric.attempts(), vec![first, second]);
}

#[test]
fn propagates_a_metric_error_and_keeps_the_series_tracked() {
    let metric = ProbeMetric::new(ProbeOutcome::Failed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    let bounds = limits(Some(1), None, None);
    tracker
        .track_series(&metric, key("m", &first), bounds, start)
        .unwrap();
    let failure = tracker
        .track_series(&metric, key("m", &second), bounds, start)
        .unwrap_err();
    assert!(matches!(failure, TrackerError::Removal { .. }));
    let retry = tracker
        .track_series(&metric, key("m", &third), bounds, start)
        .unwrap_err();
    assert!(matches!(retry, TrackerError::Removal { .. }));
    assert_eq!(metric.attempts(), vec![first.clone(), first]);
}

#[test]
fn expires_series_older_than_the_ttl_in_one_pass() {
    let metric = ProbeMetric::new(ProbeOutcome::Removed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    let bounds = limits(None, Some(5), None);
    tracker
        .track_series(&metric, key("m", &first), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &second), bounds, at(start, 5))
        .unwrap();
    assert_eq!(metric.attempts(), Vec::new());
    tracker
        .track_series(&metric, key("m", &third), bounds, at(start, 6))
        .unwrap();
    assert_eq!(metric.attempts(), vec![first]);
}

#[test]
fn runs_ttl_cleanup_only_after_the_cleanup_interval() {
    let metric = ProbeMetric::new(ProbeOutcome::Removed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    let bounds = limits(None, Some(5), Some(60));
    tracker
        .track_series(&metric, key("m", &first), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &second), bounds, at(start, 10))
        .unwrap();
    assert_eq!(metric.attempts(), Vec::new());
    tracker
        .track_series(&metric, key("m", &third), bounds, at(start, 60))
        .unwrap();
    assert_eq!(metric.attempts(), vec![first, second]);
}

#[test]
fn treats_a_zero_cleanup_interval_as_every_call() {
    let metric = ProbeMetric::new(ProbeOutcome::Removed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    let bounds = limits(None, Some(5), Some(0));
    tracker
        .track_series(&metric, key("m", &first), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &second), bounds, at(start, 10))
        .unwrap();
    tracker
        .track_series(&metric, key("m", &third), bounds, at(start, 20))
        .unwrap();
    assert_eq!(metric.attempts(), vec![first, second]);
}

#[test]
fn tracks_the_cleanup_clock_per_metric_name() {
    let metric = ProbeMetric::new(ProbeOutcome::Removed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    tracker
        .track_series(
            &metric,
            key("one", &first),
            limits(None, Some(5), Some(60)),
            start,
        )
        .unwrap();
    tracker
        .track_series(
            &metric,
            key("two", &second),
            limits(Some(5), None, None),
            start,
        )
        .unwrap();
    tracker
        .track_series(
            &metric,
            key("two", &third),
            limits(None, Some(5), Some(60)),
            at(start, 10),
        )
        .unwrap();
    assert_eq!(metric.attempts(), vec![second]);
}

#[test]
fn skips_tracking_when_no_limit_is_configured() {
    let metric = ProbeMetric::new(ProbeOutcome::Removed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let start = Instant::now();
    let first = values(&["a"]);
    let second = values(&["b"]);
    let third = values(&["c"]);
    tracker
        .track_series(&metric, key("m", &first), limits(None, None, None), start)
        .unwrap();
    let bounds = limits(Some(1), None, None);
    tracker
        .track_series(&metric, key("m", &second), bounds, start)
        .unwrap();
    tracker
        .track_series(&metric, key("m", &third), bounds, start)
        .unwrap();
    assert_eq!(metric.attempts(), vec![second]);
}

#[test]
fn reports_a_removed_child_as_gone() {
    let metric = ProbeMetric::new(ProbeOutcome::Removed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let series = values(&["a"]);
    let removed = tracker.remove_series(&metric, &series).unwrap();
    assert!(removed);
    assert_eq!(metric.attempts(), vec![series]);
}

#[test]
fn reports_an_absent_child_as_gone() {
    let metric = ProbeMetric::new(ProbeOutcome::Absent);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let series = values(&["a"]);
    let removed = tracker.remove_series(&metric, &series).unwrap();
    assert!(removed);
    assert_eq!(metric.attempts(), vec![series]);
}

#[test]
fn reports_a_refused_removal_as_present() {
    let metric = ProbeMetric::new(ProbeOutcome::Unsupported);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let series = values(&["a"]);
    let removed = tracker.remove_series(&metric, &series).unwrap();
    assert!(!removed);
}

#[test]
fn propagates_a_removal_error_from_remove_series() {
    let metric = ProbeMetric::new(ProbeOutcome::Failed);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let series = values(&["a"]);
    let failure = tracker.remove_series(&metric, &series).unwrap_err();
    assert!(matches!(failure, TrackerError::Removal { .. }));
}
