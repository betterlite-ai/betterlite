use std::sync::Arc;
use std::time::Duration;

use crate::datadog::metrics::health::async_health_check;
use crate::datadog::metrics::series::{
    DatadogMetricPoint, DatadogMetricSeries, DatadogMetricsPayload, GAUGE_TYPE,
    HEALTH_CHECK_METRIC, HEALTH_CHECK_TAG,
};
use crate::exporter_options::health::{HealthState, IntegrationHealthStatus};

use super::support::{self, FixedClock, MetricsInit, NOW, NOW_DURATION, Outcome};

const API_KEY: &str = "secret-key";

fn health_series(timestamp: i64) -> DatadogMetricsPayload {
    DatadogMetricsPayload {
        series: vec![DatadogMetricSeries {
            metric: HEALTH_CHECK_METRIC,
            kind: GAUGE_TYPE,
            points: vec![DatadogMetricPoint {
                timestamp,
                value: 1.0,
            }],
            tags: vec![HEALTH_CHECK_TAG.to_owned()],
            interval: None,
        }],
    }
}

#[tokio::test]
async fn health_without_credentials_is_healthy_and_sends_nothing() {
    let (logger, transport) = support::build(
        &support::environment(&[]),
        MetricsInit::default(),
        Vec::new(),
    );
    let status = async_health_check(&logger).await;
    assert_eq!(status, IntegrationHealthStatus::healthy());
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn health_uploads_the_test_series() {
    let environment = support::environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, transport) = support::build(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(202)],
    );
    let status = async_health_check(&logger).await;
    assert_eq!(status, IntegrationHealthStatus::healthy());
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].header("dd-api-key"), Some(API_KEY));
    let expected = serde_json::to_vec(&health_series(NOW)).expect("the health payload serializes");
    assert_eq!(calls[0].body, expected);
}

#[tokio::test]
async fn health_truncates_the_clock_to_whole_seconds() {
    let environment = support::environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, transport) = support::build_with(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(202)],
        Arc::new(FixedClock(
            NOW_DURATION
                .checked_add(Duration::from_millis(750))
                .expect("the health clock duration fits"),
        )),
        None,
    );
    let status = async_health_check(&logger).await;
    assert_eq!(status, IntegrationHealthStatus::healthy());
    let expected = serde_json::to_vec(&health_series(NOW)).expect("the health payload serializes");
    assert_eq!(transport.calls()[0].body, expected);
}

#[tokio::test]
async fn health_is_unhealthy_when_the_api_answers_a_redirect() {
    let environment = support::environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, _transport) = support::build(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(302)],
    );
    let status = async_health_check(&logger).await;
    assert_eq!(status.status, HealthState::Unhealthy);
    assert!(status.error_message.is_some());
}

#[tokio::test]
async fn health_is_unhealthy_when_the_api_answers_an_error() {
    let environment = support::environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, _transport) = support::build(
        &environment,
        MetricsInit::default(),
        vec![Outcome::Reply(500)],
    );
    let status = async_health_check(&logger).await;
    assert_eq!(status.status, HealthState::Unhealthy);
    assert!(status.error_message.is_some());
}

#[tokio::test]
async fn health_is_unhealthy_when_the_transport_fails() {
    let environment = support::environment(&[("DD_API_KEY", API_KEY)]);
    let (logger, _transport) =
        support::build(&environment, MetricsInit::default(), vec![Outcome::Network]);
    let status = async_health_check(&logger).await;
    assert_eq!(status.status, HealthState::Unhealthy);
    assert!(status.error_message.is_some());
}
