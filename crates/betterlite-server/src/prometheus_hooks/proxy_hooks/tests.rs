#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::normalize::class::Class;
use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_logging::prometheus::failure::exception::LabelException;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_prometheus::label_sanitization::sanitize_label_name;
use rust_decimal::Decimal;

use super::super::tests::support::{
    Fixture, PROXY_FAILED, PROXY_TOTAL, RecordingCatalog, fixture, fixture_with_ports,
    labelled_settings, series_label, totals,
};
use crate::prometheus_hooks::hooks::{PrometheusHookPorts, PrometheusHooks};

fn auth_from(source: &str) -> UserAPIKeyAuth {
    let map: BTreeMap<String, JsonData> = serde_json::from_str(source).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

fn auth(api_key: &str) -> UserAPIKeyAuth {
    let source = format!(r#"{{"api_key": "{api_key}", "user_id": "user-1"}}"#);
    auth_from(&source)
}

fn labelled_auth() -> UserAPIKeyAuth {
    auth_from(
        r#"{
            "api_key": "sk-secret",
            "user_id": "user-1",
            "key_alias": "alias-1",
            "team_id": "team-1",
            "team_alias": "team-alias",
            "org_id": "org-1",
            "organization_alias": "org-alias",
            "user_email": "user@example.com",
            "end_user_id": "end-user-1",
            "request_route": "/v1/chat/completions"
        }"#,
    )
}

fn request_data(model: &str) -> JsonMap {
    let source = format!(r#"{{"model": "{model}"}}"#);
    serde_json::from_str(&source).unwrap()
}

// the request a proxy-side failure carries: metadata tags, the proxy headers and the model info
fn rich_request_data() -> JsonMap {
    serde_json::from_str(
        r#"{
            "model": "gpt-4o",
            "stream": true,
            "metadata": {
                "tags": ["prod"],
                "requester_ip_address": "10.0.0.9",
                "user_agent": "test-agent/1.0",
                "model_info": {"id": ""}
            },
            "proxy_server_request": {"headers": {"user-agent": "curl/8.0"}},
            "litellm_params": {"custom_llm_provider": "openai"},
            "model_info": {"id": "request-model-id"}
        }"#,
    )
    .unwrap()
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

fn rate_limit_exception() -> LabelException {
    LabelException {
        class_name: "RateLimitError".to_owned(),
        message: Some("the call was limited".to_owned()),
        status_code: Some("429".to_owned()),
        class: Class::RateLimitError,
        code: None,
        llm_provider: None,
        category: Some("litellm_rate_limit".to_owned()),
        rate_limit_type: Some("requests".to_owned()),
        legacy_class_name: None,
    }
}

fn logged_key(auth: &UserAPIKeyAuth) -> String {
    let principal: PrincipalFacts = serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .unwrap();
    logged_api_key(&principal).unwrap().to_owned()
}

fn catalog_fixture(names: &[&str]) -> Fixture {
    catalog_ports(RecordingCatalog::new(names))
}

fn catalog_ports(catalog: RecordingCatalog) -> Fixture {
    let port: Arc<dyn RequestedModelCatalog + Send + Sync> = Arc::new(catalog);
    fixture_with_ports(
        labelled_settings(),
        PrometheusHookPorts {
            router_usage: None,
            requested_models: Some(port),
        },
    )
}

async fn failed_call(fixture: &Fixture, model: &str) {
    fixture
        .hooks
        .async_post_call_failure_hook(
            &request_data(model),
            &exception(Some("500")),
            &auth("sk-secret"),
        )
        .await;
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
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "hashed_api_key"),
        "None"
    );
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "exception_status"),
        "401"
    );
    assert_ne!(
        series_label(&fixture.registry, PROXY_FAILED, "hashed_api_key"),
        logged
    );
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
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "hashed_api_key"),
        logged
    );
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "exception_status"),
        "500"
    );
}

#[tokio::test]
async fn proxy_principal_labels_match_litellm() {
    let fixture = catalog_fixture(&["gpt-4o"]);
    let auth = labelled_auth();
    let logged = logged_key(&auth);
    fixture
        .hooks
        .async_post_call_failure_hook(&rich_request_data(), &rate_limit_exception(), &auth)
        .await;
    let registry = &fixture.registry;
    assert_eq!(
        series_label(registry, PROXY_FAILED, "end_user"),
        "end-user-1"
    );
    assert_eq!(series_label(registry, PROXY_FAILED, "user"), "user-1");
    assert_eq!(
        series_label(registry, PROXY_FAILED, "user_email"),
        "user@example.com"
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "hashed_api_key"),
        logged
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "api_key_alias"),
        "alias-1"
    );
    assert_eq!(series_label(registry, PROXY_FAILED, "team"), "team-1");
    assert_eq!(
        series_label(registry, PROXY_FAILED, "team_alias"),
        "team-alias"
    );
    assert_eq!(series_label(registry, PROXY_FAILED, "org_id"), "org-1");
    assert_eq!(
        series_label(registry, PROXY_FAILED, "org_alias"),
        "org-alias"
    );
}

#[tokio::test]
async fn proxy_failure_labels_match_litellm() {
    let fixture = catalog_fixture(&["gpt-4o"]);
    let auth = labelled_auth();
    fixture
        .hooks
        .async_post_call_failure_hook(&rich_request_data(), &rate_limit_exception(), &auth)
        .await;
    let registry = &fixture.registry;
    assert_eq!(
        series_label(registry, PROXY_FAILED, "exception_status"),
        "429"
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "exception_class"),
        "RateLimitError"
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "rate_limit_category"),
        "litellm_rate_limit"
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "rate_limit_type"),
        "requests"
    );
}

#[tokio::test]
async fn proxy_request_labels_match_litellm() {
    let fixture = catalog_fixture(&["gpt-4o"]);
    let auth = labelled_auth();
    fixture
        .hooks
        .async_post_call_failure_hook(&rich_request_data(), &rate_limit_exception(), &auth)
        .await;
    let registry = &fixture.registry;
    assert_eq!(
        series_label(registry, PROXY_FAILED, "requested_model"),
        "gpt-4o"
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "route"),
        "/v1/chat/completions"
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "client_ip"),
        "10.0.0.9"
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "user_agent"),
        "test-agent/1.0"
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "model_id"),
        "request-model-id"
    );
    assert_eq!(
        series_label(registry, PROXY_FAILED, "api_provider"),
        "openai"
    );
    assert_eq!(series_label(registry, PROXY_FAILED, "tag_prod"), "true");
}

#[tokio::test]
async fn proxy_total_labels_match_litellm() {
    let fixture = catalog_fixture(&["gpt-4o"]);
    let auth = labelled_auth();
    fixture
        .hooks
        .async_post_call_failure_hook(&rich_request_data(), &rate_limit_exception(), &auth)
        .await;
    let registry = &fixture.registry;
    // the total requests counter carries the status and the opt-in stream label
    assert_eq!(series_label(registry, PROXY_TOTAL, "status_code"), "429");
    assert_eq!(series_label(registry, PROXY_TOTAL, "stream"), "True");
    assert_eq!(
        series_label(registry, PROXY_TOTAL, "route"),
        "/v1/chat/completions"
    );
}

#[tokio::test]
async fn an_unknown_model_is_the_other_label() {
    let fixture = catalog_fixture(&["gpt-4o"]);
    fixture
        .hooks
        .async_post_call_failure_hook(
            &request_data("mystery-model"),
            &exception(Some("500")),
            &auth("sk-secret"),
        )
        .await;
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "requested_model"),
        "other"
    );
}

#[tokio::test]
async fn the_metadata_model_info_id_wins_over_the_request_data() {
    let fixture = fixture();
    let data: JsonMap = serde_json::from_str(
        r#"{
            "model": "gpt-4o",
            "metadata": {"model_info": {"id": "metadata-model-id"}},
            "model_info": {"id": "request-model-id"}
        }"#,
    )
    .unwrap();
    fixture
        .hooks
        .async_post_call_failure_hook(&data, &exception(Some("500")), &auth("sk-secret"))
        .await;
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "model_id"),
        "metadata-model-id"
    );
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

#[tokio::test]
async fn the_stream_label_is_opt_in() {
    let fixture = fixture();
    fixture
        .hooks
        .async_post_call_failure_hook(
            &rich_request_data(),
            &exception(Some("500")),
            &auth("sk-secret"),
        )
        .await;
    let names = fixture
        .registry
        .get(PROXY_TOTAL)
        .expect("the total metric is registered")
        .description
        .label_names
        .clone();
    assert!(!names.contains(&"stream".to_owned()));
}

#[tokio::test]
async fn a_recognized_model_keeps_its_name() {
    let fixture = catalog_ports(RecordingCatalog::from_sets(&["known-model"], &[], &[]));
    failed_call(&fixture, "known-model").await;
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "requested_model"),
        "known-model"
    );
}

#[tokio::test]
async fn a_team_public_model_keeps_its_name() {
    let fixture = catalog_ports(RecordingCatalog::from_sets(&[], &["shared-model"], &[]));
    failed_call(&fixture, "shared-model").await;
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "requested_model"),
        "shared-model"
    );
}

#[tokio::test]
async fn a_pattern_route_model_keeps_its_name() {
    let fixture = catalog_ports(RecordingCatalog::from_sets(&[], &[], &["wild-model"]));
    failed_call(&fixture, "wild-model").await;
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "requested_model"),
        "wild-model"
    );
}

#[tokio::test]
async fn an_empty_litellm_params_model_uses_the_request_model() {
    let fixture = fixture();
    let data: JsonMap =
        serde_json::from_str(r#"{"model": "openai/gpt-4o", "litellm_params": {"model": ""}}"#)
            .unwrap();
    fixture
        .hooks
        .async_post_call_failure_hook(&data, &exception(Some("500")), &auth("sk-secret"))
        .await;
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, "api_provider"),
        "openai"
    );
}

#[tokio::test]
async fn a_configured_header_tag_reaches_the_series() {
    let mut settings = labelled_settings();
    settings.labels.custom_tags = vec!["x-env: prod".to_owned()];
    settings.tags.extra_spend_tag_headers = vec!["x-env".to_owned()];
    let catalog: Arc<dyn RequestedModelCatalog + Send + Sync> =
        Arc::new(RecordingCatalog::new(&["gpt-4o"]));
    let fixture = fixture_with_ports(
        settings,
        PrometheusHookPorts {
            router_usage: None,
            requested_models: Some(catalog),
        },
    );
    let data: JsonMap = serde_json::from_str(
        r#"{
            "model": "gpt-4o",
            "proxy_server_request": {"headers": {"x-env": "prod"}}
        }"#,
    )
    .unwrap();
    fixture
        .hooks
        .async_post_call_failure_hook(&data, &exception(Some("500")), &auth("sk-secret"))
        .await;
    let label = sanitize_label_name("tag_x-env: prod");
    assert_eq!(
        series_label(&fixture.registry, PROXY_FAILED, &label),
        "true"
    );
}
