use crate::metric_catalog::DEFINED_PROMETHEUS_METRICS;
#[test]
fn contains_metrics() {
    assert_eq!(DEFINED_PROMETHEUS_METRICS.len(), 83);
    assert!(DEFINED_PROMETHEUS_METRICS.contains(&"litellm_in_flight_requests"));
}
