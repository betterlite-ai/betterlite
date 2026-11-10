use std::time::Duration;

use super::support::Fixture;
use crate::newrelic_metrics::NewRelicMetricsError;
use crate::newrelic_metrics::region::{EU_METRIC_ENDPOINT, US_METRIC_ENDPOINT};

#[tokio::test]
async fn init_validation_rejects_an_empty_api_key() {
    let fixture = Fixture::new();
    let Err(error) = fixture.with_key("").build_result() else {
        panic!("an empty team ingest key must fail");
    };
    assert!(matches!(error, NewRelicMetricsError::MissingApiKey));
    assert!(error.to_string().contains("newrelic_api_key"));
    assert!(error.to_string().contains("never fall back"));
}

#[tokio::test]
async fn init_validation_accepts_a_whitespace_only_api_key() {
    let fixture = Fixture::new();
    let logger = fixture.with_key(" ").build();
    assert_eq!(logger.metric_api_url, US_METRIC_ENDPOINT);
}

#[tokio::test]
async fn init_validation_fixes_the_batch_and_queue_sizes() {
    let logger = Fixture::new().build();
    assert_eq!(logger.settings.batch_size, 200);
    assert_eq!(logger.settings.max_queue_size, 10_000);
    assert_eq!(logger.settings.flush_interval, Duration::from_secs(5));
}

#[tokio::test]
async fn init_validation_resolves_the_eu_endpoint() {
    let fixture = Fixture::new().with_region("EU");
    let logger = fixture.build();
    assert_eq!(logger.metric_api_url, EU_METRIC_ENDPOINT);
}

#[tokio::test]
async fn init_validation_defaults_the_endpoint_to_us() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    assert_eq!(logger.metric_api_url, US_METRIC_ENDPOINT);
}
