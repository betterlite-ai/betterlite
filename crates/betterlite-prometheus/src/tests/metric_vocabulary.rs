#![cfg(test)]
use betterlite_prometheus::metric_catalog::DEFINED_PROMETHEUS_METRICS;

#[test]
fn contains_metrics() {
    let expected = include_str!("fixtures/metrics.txt")
        .lines()
        .collect::<Vec<_>>();
    assert_eq!(DEFINED_PROMETHEUS_METRICS, expected.as_slice());
    assert_eq!(DEFINED_PROMETHEUS_METRICS.len(), 83);
}
