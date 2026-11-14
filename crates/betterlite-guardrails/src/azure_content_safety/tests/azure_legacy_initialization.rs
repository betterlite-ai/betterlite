use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::base_builder;
use secrecy::{ExposeSecret as _, SecretString};

use super::support::{legacy, mock, threshold_map};
use crate::azure_content_safety::error::AzureContentSafetyError;
use crate::azure_content_safety::legacy::{
    LegacyAzureContentSafety, LegacyAzureContentSafetyParams,
};
use crate::azure_content_safety::legacy_thresholds::DEFAULT_THRESHOLD;
use crate::azure_content_safety::legacy_wire::TextCategory;

#[test]
fn defaults_every_category_to_severity_four() {
    let hook = legacy("http://127.0.0.1:1", None);
    assert_eq!(hook.thresholds.for_category(TextCategory::Hate), 4);
    assert_eq!(hook.thresholds.for_category(TextCategory::SelfHarm), 4);
    assert_eq!(hook.thresholds.for_category(TextCategory::Sexual), 4);
    assert_eq!(hook.thresholds.for_category(TextCategory::Violence), 4);
    assert_eq!(DEFAULT_THRESHOLD, 4);
}

#[test]
fn keeps_a_provided_threshold_and_fills_the_rest() {
    let hook = legacy("http://127.0.0.1:1", Some(threshold_map(&[("Hate", 2)])));
    assert_eq!(hook.thresholds.for_category(TextCategory::Hate), 2);
    assert_eq!(
        hook.thresholds.for_category(TextCategory::SelfHarm),
        DEFAULT_THRESHOLD
    );
    assert_eq!(
        hook.thresholds.for_category(TextCategory::Sexual),
        DEFAULT_THRESHOLD
    );
    assert_eq!(
        hook.thresholds.for_category(TextCategory::Violence),
        DEFAULT_THRESHOLD
    );
}

#[test]
fn rejects_an_unknown_threshold_category() {
    let error = LegacyAzureContentSafety::new(
        LegacyAzureContentSafetyParams {
            endpoint: "http://127.0.0.1:1".to_owned(),
            api_key: SecretString::from("azure-key".to_owned()),
            thresholds: threshold_map(&[("Toxicity", 2)]),
        },
        base_builder().build().unwrap(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AzureContentSafetyError::UnknownCategory { category } if category == "Toxicity"
    ));
}

#[test]
fn enforces_request_content() {
    const { assert!(LegacyAzureContentSafety::ENFORCES_REQUEST_CONTENT) };
}

#[test]
fn keeps_the_sdk_transport_defaults() {
    let hook = LegacyAzureContentSafety::new(
        LegacyAzureContentSafetyParams {
            endpoint: "http://127.0.0.1:1".to_owned(),
            api_key: SecretString::from("azure-key".to_owned()),
            thresholds: BTreeMap::new(),
        },
        base_builder().build().unwrap(),
    )
    .unwrap();
    assert_eq!(hook.timeout, Duration::from_secs(300));
    assert_eq!(hook.retry_backoff_factor, Duration::from_millis(800));
}

#[test]
fn keeps_the_api_key_out_of_the_debug_output() {
    let hook = legacy("http://127.0.0.1:1", None);
    assert_eq!(hook.api_key.expose_secret(), "azure-key");
    assert!(!format!("{hook:?}").contains("azure-key"));
}

#[test]
fn an_empty_thresholds_map_keeps_every_default() {
    let hook = legacy("http://127.0.0.1:1", Some(BTreeMap::new()));
    assert_eq!(
        hook.thresholds.for_category(TextCategory::Violence),
        DEFAULT_THRESHOLD
    );
}

#[tokio::test]
async fn keeps_the_endpoint_and_runs_on_the_injected_client() {
    let server = mock(200, r#"{"categoriesAnalysis":[]}"#).await;
    let hook = legacy(&server.base, None);
    assert_eq!(hook.endpoint, server.base);
    hook.test_violation("hello", None).await.unwrap();
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}
