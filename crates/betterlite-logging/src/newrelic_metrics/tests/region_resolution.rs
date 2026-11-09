use rstest::rstest;

use super::support::capture_warnings;
use crate::newrelic_metrics::region::{
    DEFAULT_REGION, EU_METRIC_ENDPOINT, US_METRIC_ENDPOINT, resolve_metric_endpoint,
};

#[test]
fn us_region_selects_the_us_endpoint() {
    assert_eq!(resolve_metric_endpoint(Some("us")), US_METRIC_ENDPOINT);
}

#[rstest]
#[case::lower("eu")]
#[case::upper("EU")]
#[case::mixed("Eu")]
fn eu_region_selects_the_eu_endpoint(#[case] region: &str) {
    assert_eq!(resolve_metric_endpoint(Some(region)), EU_METRIC_ENDPOINT);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn absent_or_empty_region_selects_the_us_endpoint(#[case] region: Option<&str>) {
    assert_eq!(resolve_metric_endpoint(region), US_METRIC_ENDPOINT);
}

#[test]
fn default_region_is_us() {
    assert_eq!(DEFAULT_REGION, "us");
}

#[rstest]
#[case::unknown("ap-south-1")]
#[case::arbitrary_url("https://evil.example/metric/v1")]
fn unknown_region_warns_and_falls_back_to_us(#[case] region: &str) {
    let (buffer, guard) = capture_warnings();
    let endpoint = resolve_metric_endpoint(Some(region));
    drop(guard);
    assert_eq!(endpoint, US_METRIC_ENDPOINT);
    let log = buffer.text();
    assert!(log.contains("unknown newrelic_region"), "{log}");
    assert!(log.contains(region), "{log}");
    assert!(log.contains("eu, us"), "{log}");
}
