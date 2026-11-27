#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::SeriesLimits;
use betterlite_models::decisions::Json;
use betterlite_models::key_digest::hashing::hash_token;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::tests::support::{
    DEPLOYMENT_SUCCESSES, LATENCY, PROXY_TOTAL, REQUESTS, SPEND, VIRTUAL_KEY_REQUESTS, dec,
    error_information, event, event_with_metadata, fixture, fixture_with_limits, labels_seen,
    observations, payload, totals,
};
use crate::prometheus_hooks::error::PrometheusHookError;

#[tokio::test]
async fn missing_payload_is_an_error() {
    let fixture = fixture();
    let event = event(payload(Decimal::ZERO));
    let result = fixture.hooks.log_success(None, &event).await;
    match result {
        Err(PrometheusHookError::MissingPayload) => {
            assert_eq!(
                PrometheusHookError::MissingPayload.to_string(),
                "standard_logging_object is required, got=None"
            );
        }
        Err(other) => panic!("expected the missing-payload error, got {other}"),
        Ok(()) => panic!("expected the missing-payload error, got Ok"),
    }
    assert!(totals(&fixture.registry, REQUESTS).is_empty());
}

#[tokio::test]
async fn invalid_key_is_skipped() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.error_information = Some(error_information("Invalid API key passed"));
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert!(totals(&fixture.registry, REQUESTS).is_empty());
    assert!(totals(&fixture.registry, PROXY_TOTAL).is_empty());
}

#[tokio::test]
async fn success_emits_every_step_and_ends_with_the_total() {
    let fixture = fixture();
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(payload(dec("2.5")))))
        .await
        .unwrap();
    assert_eq!(totals(&fixture.registry, REQUESTS), vec![Decimal::ONE]);
    assert_eq!(totals(&fixture.registry, SPEND), vec![dec("2.5")]);
    assert_eq!(observations(&fixture.registry, LATENCY), vec![1]);
    assert_eq!(
        totals(&fixture.registry, DEPLOYMENT_SUCCESSES),
        vec![Decimal::ONE]
    );
    assert_eq!(totals(&fixture.registry, PROXY_TOTAL), vec![Decimal::ONE]);
}

#[tokio::test]
async fn sk_key_is_hashed_for_the_emitters_only() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.metadata.user_api_key_hash = Some("sk-abc".to_owned());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    let hashed = hash_token("sk-abc");
    let request_labels = labels_seen(&fixture.registry, REQUESTS);
    assert!(request_labels.contains(&"sk-abc".to_owned()));
    assert!(!request_labels.contains(&hashed));
    let virtual_key_labels = labels_seen(&fixture.registry, VIRTUAL_KEY_REQUESTS);
    assert!(virtual_key_labels.contains(&hashed));
    assert!(!virtual_key_labels.contains(&"sk-abc".to_owned()));
    assert_eq!(fixture.lookups.keys(), vec![hashed]);
}

#[tokio::test]
async fn an_emitter_error_stops_the_later_steps() {
    let fixture = fixture();
    let metadata = IndexMap::from([(
        "litellm-key-remaining-requests-None".to_owned(),
        Json::Text("not-a-number".to_owned()),
    )]);
    let result = fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(
            payload(Decimal::ZERO),
            metadata,
        )))
        .await;
    assert!(matches!(result, Err(LoggerError::Delivery { .. })));
    // the usage counters ran before the rate-limit emitter, and no later step did
    assert_eq!(totals(&fixture.registry, REQUESTS), vec![Decimal::ONE]);
    assert!(observations(&fixture.registry, LATENCY).is_empty());
    assert!(totals(&fixture.registry, PROXY_TOTAL).is_empty());
}

#[tokio::test]
async fn a_full_series_table_refuses_later_series() {
    let fixture = fixture_with_limits(SeriesLimits {
        max_series: Some(0),
        ttl: None,
        cleanup_interval: None,
    });
    let result = fixture
        .hooks
        .async_log_success_event(Arc::new(event(payload(Decimal::ZERO))))
        .await;
    assert!(matches!(result, Err(LoggerError::Delivery { .. })));
    assert!(totals(&fixture.registry, REQUESTS).is_empty());
}
