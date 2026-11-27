#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::normalize::class::Class;
use betterlite_logging::prometheus::failure::exception::LabelException;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use rust_decimal::Decimal;
use serde_json::json;

use super::super::tests::support::{PROXY_FAILED, PROXY_TOTAL, fixture, labels_seen, totals};
use crate::prometheus_hooks::hooks::PrometheusHooks;

fn auth(api_key: &str) -> UserAPIKeyAuth {
    let source = format!(r#"{{"api_key": "{api_key}", "user_id": "user-1"}}"#);
    let map: BTreeMap<String, JsonData> = serde_json::from_str(&source).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

fn request_data(model: &str) -> JsonMap {
    let mut data = JsonMap::new();
    data.insert("model".to_owned(), json!(model));
    data
}

fn exception(status_code: Option<&str>) -> LabelException {
    LabelException {
        class_name: "AuthenticationError".to_owned(),
        message: Some("the call failed".to_owned()),
        status_code: status_code.map(str::to_owned),
        class: Class::default(),
        code: None,
        llm_provider: None,
        category: None,
        rate_limit_type: None,
        legacy_class_name: None,
    }
}

fn logged_key(auth: &UserAPIKeyAuth) -> String {
    let principal: PrincipalFacts = serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .unwrap();
    logged_api_key(&principal).unwrap().to_owned()
}

// the emitted series keeps its labels in the metric's registered order, so a name finds its value
fn series_label(registry: &CollectorRegistry, metric: &str, name: &str) -> String {
    let collector = registry.get(metric).expect("the metric is registered");
    let index = collector
        .description
        .label_names
        .iter()
        .position(|candidate| candidate == name)
        .expect("the label is registered");
    let series = collector.series_in_use_order();
    let (labels, _sample) = series.first().expect("the metric has a series");
    labels
        .get(index)
        .expect("the label value is emitted")
        .clone()
}

#[tokio::test]
async fn unauthorized_drops_the_key_hash() {
    let fixture = fixture();
    let auth = auth("sk-secret");
    let logged = logged_key(&auth);
    fixture
        .hooks
        .async_post_call_failure_hook(&request_data("gpt-4o"), &exception(Some("401")), &auth)
        .await;
    assert_eq!(totals(&fixture.registry, PROXY_FAILED), vec![Decimal::ONE]);
    assert_eq!(totals(&fixture.registry, PROXY_TOTAL), vec![Decimal::ONE]);
    let labels = labels_seen(&fixture.registry, PROXY_FAILED);
    assert!(!labels.contains(&logged));
    assert!(labels.contains(&"401".to_owned()));
}

#[tokio::test]
async fn other_failures_keep_the_logged_key() {
    let fixture = fixture();
    let auth = auth("sk-secret");
    let logged = logged_key(&auth);
    fixture
        .hooks
        .async_post_call_failure_hook(&request_data("gpt-4o"), &exception(Some("500")), &auth)
        .await;
    let labels = labels_seen(&fixture.registry, PROXY_FAILED);
    assert!(labels.contains(&logged));
    assert!(labels.contains(&"500".to_owned()));
}

#[tokio::test]
async fn an_unknown_status_is_the_text_none() {
    let fixture = fixture();
    let auth = auth("sk-secret");
    fixture
        .hooks
        .async_post_call_failure_hook(&request_data("gpt-4o"), &exception(None), &auth)
        .await;
    // litellm stamps str(status_code), and Python writes None for the absent code
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "exception_status"),
        "None"
    );
    assert_eq!(
        series_label(&fixture.registry, PROXY_TOTAL, "status_code"),
        "None"
    );
    assert_eq!(totals(&fixture.registry, PROXY_TOTAL), vec![Decimal::ONE]);
}

#[tokio::test]
async fn the_success_hook_emits_nothing() {
    let fixture = fixture();
    PrometheusHooks::async_post_call_success_hook().await;
    assert!(totals(&fixture.registry, PROXY_FAILED).is_empty());
    assert!(totals(&fixture.registry, PROXY_TOTAL).is_empty());
}

#[tokio::test]
async fn errors_inside_the_proxy_hook_are_swallowed() {
    let fixture = fixture();
    let auth = auth("sk-secret");
    // a tags field that is not a list fails the request-tag reader before either counter is written
    let data: JsonMap =
        serde_json::from_str(r#"{"model": "gpt-4o", "metadata": {"tags": "not-a-list"}}"#).unwrap();
    fixture
        .hooks
        .async_post_call_failure_hook(&data, &exception(Some("500")), &auth)
        .await;
    assert!(totals(&fixture.registry, PROXY_FAILED).is_empty());
    assert!(totals(&fixture.registry, PROXY_TOTAL).is_empty());
}
