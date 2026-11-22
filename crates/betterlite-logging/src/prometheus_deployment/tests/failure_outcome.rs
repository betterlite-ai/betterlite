#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::collector_factory::collector::{MetricUpdate, MetricValue};
use rust_decimal::Decimal;

use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus::failure::deployment_labels::DeploymentFailureLabels;

use super::failure::set_llm_deployment_failure_metrics;
use super::metric::DeploymentMetric;
use super::requests::FailureRequest;
use super::support::{
    FakeCatalog, ProbeGauges, call, capture_logs, exception, expected_labels, inc_amount,
    json_object, labels_of, metric_of, now, payload_with, update_of,
};

fn params(entries: &[(&str, Json)]) -> JsonObject {
    json_object(entries)
}

fn selected_payload() -> StandardLoggingPayload {
    super::support::payload_with(&[
        ("model_id", Json::Text("model-1".to_owned())),
        ("model_group", Json::Text("group-a".to_owned())),
        ("api_base", Json::Text("https://api.example".to_owned())),
    ])
}

fn total_labels(overrides: &[(&str, &str)]) -> BTreeMap<String, String> {
    expected_labels("litellm_deployment_total_requests", overrides)
}

fn failure_labels(overrides: &[(&str, &str)]) -> BTreeMap<String, String> {
    expected_labels("litellm_deployment_failure_responses", overrides)
}

#[test]
fn a_selected_failure_sets_partial_outage_then_the_counters() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let payload = selected_payload();
    let status = exception(Some("400"), None);
    let litellm_params = params(&[("custom_llm_provider", Json::Text("openai".to_owned()))]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: Some(&status),
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    assert_eq!(events.len(), 3);
    assert_eq!(metric_of(&events[0]), "litellm_deployment_state");
    assert_eq!(
        update_of(&events[0]),
        MetricUpdate::Set(MetricValue::Finite(Decimal::ONE))
    );
    assert_eq!(
        labels_of(&events[0]),
        &expected_labels(
            "litellm_deployment_state",
            &[
                ("litellm_model_name", "gpt-4o"),
                ("model_id", "model-1"),
                ("api_base", "https://api.example"),
                ("api_provider", "openai"),
            ],
        )
    );
    assert_eq!(
        metric_of(&events[1]),
        "litellm_deployment_failure_responses"
    );
    assert_eq!(update_of(&events[1]), inc_amount());
    assert_eq!(
        labels_of(&events[1]),
        &failure_labels(&[
            ("model_group", "group-a"),
            ("requested_model", "group-a"),
            ("litellm_model_name", "gpt-4o"),
            ("model_id", "model-1"),
            ("api_base", "https://api.example"),
            ("api_provider", "openai"),
            ("exception_status", "400"),
            ("exception_class", "BadRequestError"),
        ])
    );
    assert_eq!(metric_of(&events[2]), "litellm_deployment_total_requests");
    assert_eq!(
        labels_of(&events[2]),
        &total_labels(&[
            ("model_group", "group-a"),
            ("requested_model", "group-a"),
            ("litellm_model_name", "gpt-4o"),
            ("model_id", "model-1"),
            ("api_base", "https://api.example"),
            ("api_provider", "openai"),
        ])
    );
}

#[test]
fn a_selected_failure_without_an_exception_skips_the_failure_counter() {
    let gauges = ProbeGauges::new();
    let payload = selected_payload();
    let litellm_params = params(&[("custom_llm_provider", Json::Text("openai".to_owned()))]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: None,
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    assert_eq!(events.len(), 2);
    assert_eq!(metric_of(&events[0]), "litellm_deployment_state");
    assert_eq!(metric_of(&events[1]), "litellm_deployment_total_requests");
    assert!(!labels_of(&events[1]).contains_key("exception_status"));
    assert!(!labels_of(&events[1]).contains_key("exception_class"));
}

#[test]
fn proxy_rejected_before_routing_leaves_the_deployment_labels_empty() {
    let gauges = ProbeGauges::new().with_exception_class("BadRequestError");
    let payload = selected_payload();
    let status = exception(Some("400"), None);
    let litellm_params = params(&[
        ("custom_llm_provider", Json::Text("openai".to_owned())),
        ("proxy_rejected_before_routing", Json::Bool(true)),
    ]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: Some(&status),
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    assert_eq!(events.len(), 2);
    assert_eq!(
        metric_of(&events[0]),
        "litellm_deployment_failure_responses"
    );
    let labels = labels_of(&events[0]);
    assert_eq!(
        labels.get("litellm_model_name").map(String::as_str),
        Some("")
    );
    assert_eq!(labels.get("model_id").map(String::as_str), Some(""));
    assert_eq!(labels.get("api_base").map(String::as_str), Some(""));
    assert_eq!(labels.get("api_provider").map(String::as_str), Some(""));
    assert_eq!(
        labels.get("requested_model").map(String::as_str),
        Some("gpt-4o")
    );
    assert_eq!(
        labels.get("model_group").map(String::as_str),
        Some("group-a")
    );
}

#[test]
fn a_failure_without_a_model_id_is_rejected_before_routing() {
    let gauges = ProbeGauges::new();
    let payload = super::support::payload_with(&[
        ("model_group", Json::Text("group-a".to_owned())),
        ("api_base", Json::Text(String::new())),
    ]);
    let litellm_params = params(&[]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: None,
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    assert_eq!(events.len(), 1);
    let labels = labels_of(&events[0]);
    assert_eq!(
        labels.get("litellm_model_name").map(String::as_str),
        Some("")
    );
    assert_eq!(
        labels.get("requested_model").map(String::as_str),
        Some("gpt-4o")
    );
    assert_eq!(
        labels.get("model_group").map(String::as_str),
        Some("group-a")
    );
}

#[test]
fn the_rejected_requested_model_is_bounded_against_the_catalog() {
    let gauges = ProbeGauges::new();
    let catalog = FakeCatalog::rejecting();
    let payload = super::support::payload_with(&[("api_base", Json::Text(String::new()))]);
    let litellm_params = params(&[]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("unknown-model"),
            exception: None,
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: Some(&catalog),
            now: now(),
        },
    );
    let events = gauges.updates();
    let labels = labels_of(&events[0]);
    assert_eq!(
        labels.get("requested_model").map(String::as_str),
        Some("other")
    );
    assert_eq!(labels.get("model_group").map(String::as_str), Some("None"));
    assert_eq!(catalog.calls(), vec!["unknown-model".to_owned()]);
}

#[test]
fn model_id_falls_back_to_litellm_metadata_then_metadata() {
    let internal = ProbeGauges::new();
    let payload = super::support::payload_with(&[("api_base", Json::Text(String::new()))]);
    let litellm_metadata = Json::Object(json_object(&[(
        "model_info",
        Json::Object(json_object(&[("id", Json::Text("internal-id".to_owned()))])),
    )]));
    let litellm_params = params(&[("litellm_metadata", litellm_metadata)]);
    set_llm_deployment_failure_metrics(
        &internal,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: None,
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = internal.updates();
    let labels = labels_of(&events[0]);
    assert_eq!(
        labels.get("model_id").map(String::as_str),
        Some("internal-id")
    );

    let caller = ProbeGauges::new();
    let empty_internal = Json::Object(json_object(&[(
        "model_info",
        Json::Object(json_object(&[])),
    )]));
    let caller_info = Json::Object(json_object(&[(
        "model_info",
        Json::Object(json_object(&[("id", Json::Text("caller-id".to_owned()))])),
    )]));
    let caller_params = params(&[
        ("litellm_metadata", empty_internal),
        ("metadata", caller_info),
    ]);
    set_llm_deployment_failure_metrics(
        &caller,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: None,
            litellm_params: &caller_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let caller_events = caller.updates();
    let caller_labels = labels_of(&caller_events[0]);
    assert_eq!(
        caller_labels.get("model_id").map(String::as_str),
        Some("caller-id")
    );
}

#[test]
fn model_group_falls_back_to_litellm_metadata_then_metadata() {
    let gauges = ProbeGauges::new();
    let payload = super::support::payload_with(&[
        ("model_id", Json::Text("model-1".to_owned())),
        ("api_base", Json::Text(String::new())),
    ]);
    let caller_metadata = Json::Object(json_object(&[(
        "model_group",
        Json::Text("caller-group".to_owned()),
    )]));
    let internal_metadata = Json::Object(json_object(&[(
        "model_group",
        Json::Text("internal-group".to_owned()),
    )]));
    let litellm_params = params(&[
        ("litellm_metadata", internal_metadata),
        ("metadata", caller_metadata),
    ]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: None,
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    let labels = labels_of(&events[1]);
    assert_eq!(
        labels.get("model_group").map(String::as_str),
        Some("internal-group")
    );
    assert_eq!(
        labels.get("requested_model").map(String::as_str),
        Some("internal-group")
    );
}

#[test]
fn identity_labels_fall_through_a_falsy_b3880_result_to_the_payload_metadata() {
    let override_labels = DeploymentFailureLabels {
        api_key_alias: Some(String::new()),
        user_email: None,
        team: Some("auth-team".to_owned()),
        team_alias: None,
        hashed_api_key: None,
        client_ip: None,
        user_agent: None,
    };
    let gauges = ProbeGauges::new().with_failure_labels(override_labels);
    let metadata = Json::Object(json_object(&[
        ("user_api_key_alias", Json::Text("payload-alias".to_owned())),
        (
            "user_api_key_team_id",
            Json::Text("payload-team".to_owned()),
        ),
        (
            "user_api_key_team_alias",
            Json::Text("payload-team-alias".to_owned()),
        ),
        ("user_api_key_hash", Json::Text("payload-hash".to_owned())),
        ("requester_ip_address", Json::Text("1.2.3.4".to_owned())),
        ("user_agent", Json::Text("test-agent".to_owned())),
    ]));
    let payload = super::support::payload_with(&[
        ("model_id", Json::Text("model-1".to_owned())),
        ("api_base", Json::Text(String::new())),
        ("metadata", metadata),
    ]);
    let litellm_params = params(&[]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: None,
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    let labels = labels_of(&events[1]);
    assert_eq!(
        labels.get("api_key_alias").map(String::as_str),
        Some("payload-alias")
    );
    assert_eq!(labels.get("team").map(String::as_str), Some("auth-team"));
    assert_eq!(
        labels.get("team_alias").map(String::as_str),
        Some("payload-team-alias")
    );
    assert_eq!(
        labels.get("hashed_api_key").map(String::as_str),
        Some("payload-hash")
    );
    assert_eq!(labels.get("client_ip").map(String::as_str), Some("1.2.3.4"));
    assert_eq!(
        labels.get("user_agent").map(String::as_str),
        Some("test-agent")
    );
    assert!(gauges.calls().contains(&call("failure_labels", "")));
}

#[test]
fn the_shared_default_labels_read_the_auth_facts() {
    let gauges = ProbeGauges::new();
    let defaults = gauges.defaults();
    let payload = selected_payload();
    let status = exception(Some("400"), None);
    let auth = Json::Object(json_object(&[
        ("key_alias", Json::Text("auth-alias".to_owned())),
        ("team_id", Json::Text("auth-team".to_owned())),
        ("team_alias", Json::Text("auth-team-alias".to_owned())),
        ("api_key", Json::Text("auth-hash".to_owned())),
    ]));
    let metadata = Json::Object(json_object(&[("user_api_key_auth", auth)]));
    let litellm_params = params(&[("metadata", metadata)]);
    set_llm_deployment_failure_metrics(
        &defaults,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: Some(&status),
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    let labels = labels_of(&events[1]);
    assert_eq!(
        labels.get("api_key_alias").map(String::as_str),
        Some("auth-alias")
    );
    assert_eq!(labels.get("team").map(String::as_str), Some("auth-team"));
    assert_eq!(
        labels.get("team_alias").map(String::as_str),
        Some("auth-team-alias")
    );
    assert_eq!(
        labels.get("hashed_api_key").map(String::as_str),
        Some("auth-hash")
    );
    assert_eq!(
        labels.get("exception_class").map(String::as_str),
        Some("BadRequestError")
    );
}

#[test]
fn the_shared_default_labels_bound_the_rejected_models() {
    let gauges = ProbeGauges::new();
    let defaults = gauges.defaults();
    let payload = payload_with(&[
        ("model_group", Json::Text("group-a".to_owned())),
        ("api_base", Json::Text(String::new())),
    ]);
    let litellm_params = params(&[]);
    set_llm_deployment_failure_metrics(
        &defaults,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: None,
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    let labels = labels_of(&events[0]);
    assert_eq!(
        labels.get("requested_model").map(String::as_str),
        Some("gpt-4o")
    );
    assert_eq!(
        labels.get("model_group").map(String::as_str),
        Some("group-a")
    );
}

#[test]
fn the_shared_default_filter_skips_only_invalid_key_failures() {
    let payload = selected_payload();
    let litellm_params = params(&[]);
    let unauthorized = exception(Some("401"), None);
    let invalid = ProbeGauges::new();
    let invalid_defaults = invalid.defaults();
    set_llm_deployment_failure_metrics(
        &invalid_defaults,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: Some(&unauthorized),
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    assert!(invalid.updates().is_empty());

    let bad_request = exception(Some("400"), None);
    let ordinary = ProbeGauges::new();
    let ordinary_defaults = ordinary.defaults();
    set_llm_deployment_failure_metrics(
        &ordinary_defaults,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: Some(&bad_request),
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    assert_eq!(ordinary.updates().len(), 3);
}

#[test]
fn configured_custom_tags_project_the_payload_tags() {
    let gauges = ProbeGauges::new().with_custom_tags(&["prod"]);
    let payload = payload_with(&[
        ("model_id", Json::Text("model-1".to_owned())),
        ("api_base", Json::Text(String::new())),
        (
            "request_tags",
            Json::List(vec![Json::Text("prod".to_owned())]),
        ),
    ]);
    let litellm_params = params(&[]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: None,
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let event = gauges
        .updates()
        .into_iter()
        .find(|event| metric_of(event) == "litellm_deployment_total_requests")
        .expect("the total counter is written");
    assert_eq!(
        labels_of(&event).get("tag_prod").map(String::as_str),
        Some("true")
    );
}

fn exception_status_label(status: Option<&str>, code: Option<&str>) -> Option<String> {
    let gauges = ProbeGauges::new();
    let payload = super::support::payload_with(&[("api_base", Json::Text(String::new()))]);
    let litellm_params = params(&[]);
    let raised = exception(status, code);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: Some(&raised),
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    labels_of(&events[0])
        .get("exception_status")
        .map(String::as_str)
        .map(str::to_owned)
}

#[test]
fn exception_status_uses_the_status_code_first() {
    assert_eq!(
        exception_status_label(Some("401"), Some("bad")),
        Some("401".to_owned())
    );
}

#[test]
fn exception_status_falls_back_to_the_code() {
    assert_eq!(
        exception_status_label(Some("None"), Some("400")),
        Some("400".to_owned())
    );
    assert_eq!(
        exception_status_label(Some(""), Some("bad")),
        Some("bad".to_owned())
    );
}

#[test]
fn exception_status_is_the_text_none_when_everything_is_missing() {
    assert_eq!(exception_status_label(None, None), Some("None".to_owned()));
}

#[test]
fn an_invalid_key_failure_records_nothing() {
    let gauges = ProbeGauges::new().skipping_invalid_keys();
    let payload = selected_payload();
    let status = exception(Some("400"), None);
    let litellm_params = params(&[]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: Some(&status),
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    assert!(gauges.updates().is_empty());
    assert!(gauges.calls().contains(&call("skip_invalid_key", "")));
}

#[test]
fn a_failure_error_is_logged_at_debug_and_stops_the_later_writes() {
    let gauges = ProbeGauges::new().failing_on(DeploymentMetric::FailureResponses);
    let payload = selected_payload();
    let status = exception(Some("400"), None);
    let litellm_params = params(&[]);
    let (captured, _guard) = capture_logs(tracing::Level::DEBUG);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: Some(&status),
            litellm_params: &litellm_params,
            payload: Some(&payload),
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    assert_eq!(events.len(), 1);
    assert_eq!(metric_of(&events[0]), "litellm_deployment_state");
    assert!(
        captured
            .text()
            .contains("Prometheus Error: set_llm_deployment_failure_metrics. Exception occured -")
    );
}

#[test]
fn a_failure_without_a_payload_uses_the_default_empty_payload() {
    let gauges = ProbeGauges::new();
    let litellm_params = params(&[]);
    set_llm_deployment_failure_metrics(
        &gauges,
        &FailureRequest {
            model: Some("gpt-4o"),
            exception: None,
            litellm_params: &litellm_params,
            payload: None,
            catalog: None,
            now: now(),
        },
    );
    let events = gauges.updates();
    assert_eq!(events.len(), 1);
    assert_eq!(metric_of(&events[0]), "litellm_deployment_total_requests");
    assert_eq!(
        labels_of(&events[0]).get("api_base").map(String::as_str),
        Some("")
    );
}
