#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use super::dispatch::{DispatchSettings, perform_health_checks};
use super::endpoints::separate_results;
use super::ports::{ProbeFailure, ProbeResponse};
use super::probe::ProbeOutcome;
use super::support::{
    FakeModels, FakeProvider, capture_logs, context, cycle, deployment, deployment_with_info,
    failure, number, object, plain_request, policy, prepare_all, text,
};

fn leaf_with_secret() -> DeploymentSpec {
    deployment(
        "leaf",
        "leaf-id",
        r#"{"model":"openai/leaf","api_base":"https://leaf.example","api_key":"sk-secret"}"#,
    )
}

#[test]
fn a_healthy_row_carries_the_params_headers_and_id() {
    let response = ProbeResponse {
        remaining_requests: Some("9".to_owned()),
        remaining_tokens: Some("900".to_owned()),
        region: Some("westus".to_owned()),
    };
    let result = separate_results(
        &[leaf_with_secret()],
        &[ProbeOutcome::Healthy(response)],
        true,
    );
    assert!(result.unhealthy_endpoints.is_empty());
    assert!(result.exceptions_by_model_id.is_empty());
    let endpoint = result
        .healthy_endpoints
        .first()
        .expect("the healthy probe is reported");
    assert_eq!(endpoint.get("model"), Some(&text("openai/leaf")));
    assert_eq!(
        endpoint.get("api_base"),
        Some(&text("https://leaf.example"))
    );
    assert_eq!(
        endpoint.get("x-ratelimit-remaining-requests"),
        Some(&text("9"))
    );
    assert_eq!(
        endpoint.get("x-ratelimit-remaining-tokens"),
        Some(&text("900"))
    );
    assert_eq!(endpoint.get("x-ms-region"), Some(&text("westus")));
    assert_eq!(endpoint.get("model_id"), Some(&text("leaf-id")));
    assert_eq!(endpoint.get("api_key"), None);
    assert_eq!(endpoint.get("error"), None);
}

#[test]
fn a_failed_row_holds_the_exception_status_but_not_the_exception() {
    let exception = failure(429, "rate limited");
    let result = separate_results(
        &[leaf_with_secret()],
        &[ProbeOutcome::Failed(exception.clone())],
        true,
    );
    assert!(result.healthy_endpoints.is_empty());
    let endpoint = result
        .unhealthy_endpoints
        .first()
        .expect("the failed probe is reported");
    assert_eq!(endpoint.get("error"), Some(&text("rate limited")));
    assert_eq!(endpoint.get("exception_status"), Some(&number(429)));
    assert_eq!(endpoint.get("exception"), None);
    assert_eq!(endpoint.get("model_id"), Some(&text("leaf-id")));
    assert_eq!(
        result.exceptions_by_model_id.get("leaf-id"),
        Some(&exception)
    );
}

#[test]
fn minimal_details_hide_the_error_and_the_provider_fields() {
    let result = separate_results(
        &[leaf_with_secret()],
        &[ProbeOutcome::Failed(failure(500, "boom"))],
        false,
    );
    let endpoint = result
        .unhealthy_endpoints
        .first()
        .expect("the failed probe is reported");
    assert_eq!(endpoint.get("model"), Some(&text("openai/leaf")));
    assert_eq!(endpoint.get("model_id"), Some(&text("leaf-id")));
    assert_eq!(endpoint.get("exception_status"), Some(&number(500)));
    assert_eq!(endpoint.get("error"), None);
    assert_eq!(endpoint.get("api_base"), None);
}

#[test]
fn a_row_without_an_id_keeps_its_exception_off_the_map() {
    let anonymous = deployment("leaf", "", r#"{"model":"openai/leaf"}"#);
    let result = separate_results(
        &[anonymous],
        &[ProbeOutcome::Failed(failure(500, "boom"))],
        true,
    );
    let endpoint = result
        .unhealthy_endpoints
        .first()
        .expect("the failed probe is reported");
    assert_eq!(endpoint.get("model_id"), None);
    assert_eq!(endpoint.get("exception_status"), None);
    assert!(result.exceptions_by_model_id.is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_dispatch_summary_counts_the_run() {
    let (buffer, _guard) = capture_logs();
    let rows = [
        deployment("a", "a-id", r#"{"model":"openai/a"}"#),
        deployment("b", "b-id", r#"{"model":"openai/b"}"#),
        deployment("c", "c-id", r#"{"model":"openai/c"}"#),
    ];
    let prepared = prepare_all(&rows);
    let provider = FakeProvider::new();
    let context = context("cycle-3", "background");
    let settings = DispatchSettings {
        provider: &provider,
        details: true,
        max_concurrency: Some(2),
        instrumentation: &context,
    };
    let result = perform_health_checks(&rows, &prepared, &settings).await;
    assert_eq!(result.healthy_endpoints.len(), 3);
    let logs = buffer.text();
    assert!(logs.contains("health_check_dispatch_summary"), "{logs}");
    assert!(logs.contains("source=background"), "{logs}");
    assert!(logs.contains("cycle_id=cycle-3"), "{logs}");
    assert!(logs.contains("mode=bounded"), "{logs}");
    assert!(logs.contains("model_count=3"), "{logs}");
    assert!(logs.contains("max_concurrency=2"), "{logs}");
    assert!(logs.contains("peak_in_flight=2"), "{logs}");
    assert!(logs.contains("thread_count="), "{logs}");
    assert!(logs.contains("rss_mb="), "{logs}");
}

#[tokio::test(start_paused = true)]
async fn a_disabled_context_logs_no_summary() {
    let (buffer, _guard) = capture_logs();
    let rows = [deployment("a", "a-id", r#"{"model":"openai/a"}"#)];
    let prepared = prepare_all(&rows);
    let provider = FakeProvider::new();
    let context = super::telemetry::InstrumentationContext::default();
    let settings = DispatchSettings {
        provider: &provider,
        details: true,
        max_concurrency: None,
        instrumentation: &context,
    };
    let result = perform_health_checks(&rows, &prepared, &settings).await;
    assert_eq!(result.healthy_endpoints.len(), 1);
    assert_eq!(buffer.text(), "");
}

#[tokio::test]
async fn a_probe_that_raises_before_the_provider_keeps_its_params_only() {
    let rows = [deployment_with_info(
        "leaf",
        "leaf-id",
        r#"{"model":"openai/leaf"}"#,
        r#"{"health_check_max_tokens":"plenty"}"#,
    )];
    let provider = FakeProvider::new();
    let result = cycle(
        plain_request(&rows),
        &provider,
        &FakeModels::default(),
        &policy(),
    )
    .await
    .expect("the cycle runs");
    assert!(result.healthy_endpoints.is_empty());
    let endpoint = result
        .unhealthy_endpoints
        .first()
        .expect("the raised probe is reported");
    assert_eq!(endpoint.get("model"), Some(&text("openai/leaf")));
    assert_eq!(endpoint.get("model_id"), Some(&text("leaf-id")));
    assert_eq!(endpoint.get("exception_status"), Some(&number(500)));
    assert_eq!(endpoint.get("error"), None);
    assert!(result.exceptions_by_model_id.contains_key("leaf-id"));
    assert!(provider.calls().is_empty());
}

#[test]
fn a_failure_row_keeps_the_raw_request_when_the_probe_logged_one() {
    let raw = Json::Object(object(vec![("model", text("openai/leaf"))]));
    let exception = ProbeFailure {
        message: "bad request".to_owned(),
        status_code: 400,
        raw_request_typed_dict: Some(raw.clone()),
    };
    let result = separate_results(
        &[deployment("leaf", "leaf-id", r#"{"model":"openai/leaf"}"#)],
        &[ProbeOutcome::Failed(exception)],
        true,
    );
    let endpoint = result
        .unhealthy_endpoints
        .first()
        .expect("the failed probe is reported");
    assert_eq!(endpoint.get("raw_request_typed_dict"), Some(&raw));
}
