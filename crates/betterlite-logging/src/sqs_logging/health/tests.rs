#![cfg(test)]

use std::sync::Arc;

use crate::exporter_options::health::HealthState;
use crate::sqs_logging::tests::support::{FailingHealth, Harness, RecordingDelivery, logged};

#[tokio::test]
async fn health_check_is_healthy_after_a_successful_send() {
    let (logger, records) = Harness::standard().build();
    let status = logger.async_health_check().await;
    assert_eq!(status.status, HealthState::Healthy);
    assert!(status.error_message.is_none());
    records.wait_for(1).await;
    assert!(records.body(0).contains("Action=SendMessage"));
    assert!(records.body(0).contains("sqs-health-check"));
}

#[tokio::test]
async fn health_check_stays_healthy_when_the_send_fails() {
    let mut harness = Harness::standard();
    harness.delivery = RecordingDelivery::failing();
    let (logger, records) = harness.build();
    let (status, log) = logged(logger.async_health_check()).await;
    assert_eq!(status.status, HealthState::Healthy);
    assert!(log.contains("Error sending to SQS"));
    assert_eq!(records.count(), 1);
}

#[tokio::test]
async fn health_check_reports_a_payload_factory_error() {
    let mut harness = Harness::standard();
    harness.health_payload = Arc::new(FailingHealth);
    let (logger, records) = harness.build();
    let status = logger.async_health_check().await;
    assert_eq!(status.status, HealthState::Unhealthy);
    assert!(
        status
            .error_message
            .expect("the error is reported")
            .contains("line 1")
    );
    assert_eq!(records.count(), 0);
}
